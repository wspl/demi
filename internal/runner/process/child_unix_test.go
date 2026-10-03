//go:build darwin || linux

package process

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/runnerwire"
	"golang.org/x/sys/unix"
)

// These scenarios use real OS processes, normally below one second each.
// Deadlines only guard hangs; synchronization uses IO events and process exit.
func childOptions(t *testing.T, command string, args ...string) SpawnOptions {
	t.Helper()
	return SpawnOptions{
		Command:      command,
		Args:         args,
		Cwd:          t.TempDir(),
		Env:          map[string]string{"PATH": "/usr/bin:/bin"},
		ProcessGroup: true,
	}
}

func childContext(t *testing.T) context.Context {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
	t.Cleanup(cancel)
	return ctx
}

func spawnChild(t *testing.T, options SpawnOptions) *Child {
	t.Helper()
	child, err := Spawn(childContext(t), options)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		child.Cancel()
		child.Wait(context.Background())
	})
	return child
}

func requireSuccess(t *testing.T, exit Exit) {
	t.Helper()
	if exit.Code == nil || *exit.Code != 0 || exit.Error != nil {
		t.Fatalf("exit = %+v", exit)
	}
}

func TestChildStreamsBinaryAndReaps(t *testing.T) {
	child := spawnChild(t, childOptions(t, "/bin/cat"))
	want := []byte{0, 255, 128, 10}
	child.Input <- Input{Bytes: want}
	select {
	case chunk := <-child.Output:
		if chunk.Stream != runnerwire.Stdout || !bytes.Equal(chunk.Bytes, want) {
			t.Fatalf("chunk = %+v", chunk)
		}
	case <-child.command.ctx.Done():
		t.Fatal("output did not arrive before input EOF")
	}
	child.Input <- Input{}
	for chunk := range child.Output {
		t.Fatalf("unexpected output after EOF: %+v", chunk)
	}
	exit := child.Wait(childContext(t))
	requireSuccess(t, exit)
	requireSuccess(t, child.Wait(childContext(t)))
	if err := unix.Kill(int(child.PID), 0); !errors.Is(err, unix.ESRCH) {
		t.Fatalf("child was not reaped: %v", err)
	}
}

func TestChildCancellationInterruptsBackpressure(t *testing.T) {
	child := spawnChild(t, childOptions(t, "/usr/bin/yes"))
	select {
	case <-child.Output:
	case <-child.command.ctx.Done():
		t.Fatal("no output")
	}
	// Observe the bounded output queue actually fill, rather than sleeping and
	// assuming that the writer reached backpressure.
	for len(child.Output) != cap(child.Output) {
		if err := child.command.ctx.Err(); err != nil {
			t.Fatal(err)
		}
		runtime.Gosched()
	}
	child.Cancel()
	exit := child.Wait(childContext(t))
	if exit.Signal == nil || *exit.Signal != "SIGKILL" || exit.Error != nil {
		t.Fatalf("exit = %+v", exit)
	}
	if !child.IsCancelled() {
		t.Fatal("cancellation not recorded")
	}
}

func TestChildCancellationKillsDescendants(t *testing.T) {
	child := spawnChild(t, childOptions(t, "/bin/sh", "-c", "sleep 30 & printf '%s\\n' $!; wait"))
	var output OutputChunk
	select {
	case output = <-child.Output:
	case <-child.command.ctx.Done():
		t.Fatal("no descendant PID")
	}
	pid, err := strconv.Atoi(strings.TrimSpace(string(output.Bytes)))
	if err != nil {
		t.Fatal(err)
	}
	child.Cancel()
	child.Wait(childContext(t))
	ctx := childContext(t)
	// A reparented zombie can remain until init reaps it. Poll only the observed
	// process state, with a hang deadline, never a fixed settling interval.
	for {
		if err := unix.Kill(pid, 0); errors.Is(err, unix.ESRCH) {
			break
		}
		if err := ctx.Err(); err != nil {
			t.Fatalf("descendant %d remains: %v", pid, err)
		}
		runtime.Gosched()
	}
}

func TestSpawnFailureClassification(t *testing.T) {
	for _, tc := range []struct {
		name, command, cwd string
		kind               runnerwire.SpawnErrorKind
	}{
		{
			name:    "executable",
			command: "/definitely-not-a-demi-test-program",
			kind:    runnerwire.SpawnErrorKindExecutableNotFound,
		},
		{
			name:    "directory",
			command: "/bin/true",
			cwd:     filepath.Join(t.TempDir(), "missing"),
			kind:    runnerwire.SpawnErrorKindCwdUnusable,
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			options := childOptions(t, tc.command)
			if tc.cwd != "" {
				options.Cwd = tc.cwd
			}
			_, err := Spawn(childContext(t), options)
			var failure *SpawnFailure
			if !errors.As(err, &failure) || failure.Kind != tc.kind {
				t.Fatalf("failure = %v", err)
			}
			if !errors.Is(err, os.ErrNotExist) {
				t.Fatalf("cause was lost: %v", err)
			}
		})
	}
}

func TestBootstrapAttributesAndIdentity(t *testing.T) {
	directory := t.TempDir()
	target := filepath.Join(directory, "created")
	marker := filepath.Join(directory, "extra")
	extra, err := os.Create(marker)
	if err != nil {
		t.Fatal(err)
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() { _ = extra.Close() }()
	var limits unix.Rlimit
	if err := unix.Getrlimit(unix.RLIMIT_NOFILE, &limits); err != nil {
		t.Fatal(err)
	}
	mask := uint32(0o077)
	cmd := exec.Command(
		"/bin/sh",
		"-c",
		`printf '%s\n' "$$"; umask; ulimit -Sn; printf extra >&3; : > "$1"; printf '%s\n' "$DEMI_BOOTSTRAP_TEST"`,
		"fixture",
		target,
	)
	cmd.Env = append(os.Environ(), "DEMI_BOOTSTRAP_TEST=untouched")
	cmd.ExtraFiles = []*os.File{extra}
	var output bytes.Buffer
	cmd.Stdout = &output
	wrapped := Wrap(
		cmd,
		true,
		ChildAttributes{
			Umask:  &mask,
			Limits: []ResourceLimit{{Resource: unix.RLIMIT_NOFILE, Soft: 128, Hard: limits.Max}},
		},
	)
	if err := wrapped.Start(childContext(t)); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = wrapped.Kill()
		wrapped.Wait(context.Background())
	})
	requireSuccess(t, wrapped.Wait(childContext(t)))
	lines := strings.Split(strings.TrimSpace(output.String()), "\n")
	if len(lines) != 4 || lines[0] != strconv.Itoa(int(wrapped.PID())) || strings.TrimLeft(lines[1], "0") != "77" ||
		lines[2] != "128" ||
		lines[3] != "untouched" {
		t.Fatalf("bootstrap output %q", output.String())
	}
	stat, err := os.Stat(target)
	if err != nil {
		t.Fatal(err)
	}
	if stat.Mode().Perm() != 0o600 {
		t.Fatalf("mode %o", stat.Mode().Perm())
	}
	data, err := os.ReadFile(marker)
	if err != nil || string(data) != "extra" {
		t.Fatalf("extra descriptor: %q %v", data, err)
	}
	var unchanged unix.Rlimit
	if err := unix.Getrlimit(unix.RLIMIT_NOFILE, &unchanged); err != nil {
		t.Fatal(err)
	}
	if unchanged != limits {
		t.Fatalf("runner limit changed: %+v -> %+v", limits, unchanged)
	}
}

func TestBootstrapStartFailures(t *testing.T) {
	mask := uint32(0o077)
	for _, tc := range []struct {
		name, path string
		attrs      ChildAttributes
		want       error
	}{
		{"missing", "/definitely-not-a-demi-test-program", ChildAttributes{Umask: &mask}, unix.ENOENT},
		{
			"invalid limit",
			"/bin/true",
			ChildAttributes{Limits: []ResourceLimit{{Resource: unix.RLIMIT_NOFILE, Soft: 9, Hard: 8}}},
			unix.EINVAL,
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			command := Wrap(exec.Command(tc.path), true, tc.attrs)
			err := command.Start(childContext(t))
			if !errors.Is(err, tc.want) {
				t.Fatalf("start error %v, want %v", err, tc.want)
			}
		})
	}
}

func TestGroupLeaderExitClosesDescendantOutput(t *testing.T) {
	child := spawnChild(t, childOptions(t, "/bin/sh", "-c", "sleep 30 & printf done"))
	var data []byte
	for part := range child.Output {
		data = append(data, part.Bytes...)
	}
	requireSuccess(t, child.Wait(childContext(t)))
	if string(data) != "done" {
		t.Fatalf("output %q", data)
	}
}

func TestCommandWaitCancellationInterruptsIO(t *testing.T) {
	input, writer := io.Pipe()
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() { _ = writer.Close() }()
	cmd := exec.Command("/bin/cat")
	cmd.Stdin = input
	command := Wrap(cmd, true, ChildAttributes{})
	if err := command.Start(childContext(t)); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	exit := command.Wait(ctx)
	if exit.Signal == nil || *exit.Signal != "SIGKILL" {
		t.Fatalf("exit %+v", exit)
	}
}

func TestCommandOutputFailureKillsChild(t *testing.T) {
	cmd := exec.Command("/usr/bin/yes")
	sentinel := fmt.Errorf("output failed")
	cmd.Stdout = failingOutput{err: sentinel}
	command := Wrap(cmd, true, ChildAttributes{})
	if err := command.Start(childContext(t)); err != nil {
		t.Fatal(err)
	}
	exit := command.Wait(childContext(t))
	if exit.Error == nil || !strings.Contains(*exit.Error, sentinel.Error()) {
		t.Fatalf("exit %+v", exit)
	}
}

type failingOutput struct{ err error }

func (f failingOutput) Write([]byte) (int, error) { return 0, f.err }

func TestSpawnUsesJobPATHAndEnvironment(t *testing.T) {
	options := childOptions(t, "job-program")
	if err := os.WriteFile(
		filepath.Join(options.Cwd, "job-program"),
		[]byte("#!/bin/sh\nprintf '%s' \"$JOB_VALUE\"\n"),
		0o700,
	); err != nil {
		t.Fatal(err)
	}
	options.Env = map[string]string{"PATH": ".", "JOB_VALUE": "job environment"}
	child := spawnChild(t, options)
	var output []byte
	for part := range child.Output {
		output = append(output, part.Bytes...)
	}
	requireSuccess(t, child.Wait(childContext(t)))
	if string(output) != "job environment" {
		t.Fatalf("output %q", output)
	}
}

// cancelAtStatus reproduces cancellation between disarming the handshake's
// callback and examining its result, without a scheduler-dependent race.
type cancelAtStatus struct {
	context.Context
	cancel context.CancelFunc
}

func (c cancelAtStatus) Err() error {
	c.cancel()
	return c.Context.Err()
}

func TestBootstrapCancellationAfterExecAcknowledgement(t *testing.T) {
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = reader.Close() }()
	cmd := exec.Command("/bin/cat")
	cmd.Stdin = reader
	mask := uint32(0o077)
	owner, cancel := context.WithCancel(t.Context())
	defer cancel()
	done := make(chan error, 1)
	joined := make(chan struct{})
	go func() {
		defer close(joined)
		started, err := startPlatform(
			cancelAtStatus{Context: owner, cancel: cancel},
			cmd,
			true,
			ChildAttributes{Umask: &mask},
			&platformGroup{},
		)
		if started != nil {
			_ = started.Process.Kill()
			_ = started.Wait()
		}
		done <- err
	}()
	defer func() {
		_ = writer.Close()
		<-joined
	}()
	deadline, stop := context.WithTimeout(t.Context(), time.Second)
	defer stop()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("cancelled start: %v", err)
		}
	case <-deadline.Done():
		t.Fatal("cancelled bootstrap waited for the running program")
	}
}

func TestCommandCombinedOutputKeepsWriteOrder(t *testing.T) {
	var output bytes.Buffer
	cmd := exec.Command("/bin/sh", "-c", "printf first; printf second >&2; printf third")
	cmd.Stdout = &output
	cmd.Stderr = &output
	command := Wrap(cmd, true, ChildAttributes{})
	if err := command.Start(childContext(t)); err != nil {
		t.Fatal(err)
	}
	requireSuccess(t, command.Wait(childContext(t)))
	if output.String() != "firstsecondthird" {
		t.Fatalf("combined output %q", output.String())
	}
}
