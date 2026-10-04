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

	"github.com/wspl/demi/internal/runnerwire"
	"golang.org/x/sys/unix"
)

// These scenarios use real OS processes, normally below one second each.
// Synchronization uses IO events and process exit; go test -timeout guards hangs.
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

func spawnChild(t *testing.T, options SpawnOptions) *Child {
	t.Helper()
	child, err := Spawn(t.Context(), options)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		child.Cancel()
		_, _ = child.Wait(context.Background())
	})
	return child
}

// unsuccessfulExit describes an exit that is not a clean zero status.
func unsuccessfulExit(exit Exit, err error) error {
	if exit.Code == nil || *exit.Code != 0 || err != nil {
		return fmt.Errorf("exit = %+v, err = %v", exit, err)
	}
	return nil
}

func TestChildStreamsBinaryAndReaps(t *testing.T) {
	child := spawnChild(t, childOptions(t, "/bin/cat"))
	want := []byte{0, 255, 128, 10}
	child.Input <- Input{Bytes: want}
	// The output arrives before input EOF.
	if chunk := <-child.Output; chunk.Stream != runnerwire.Stdout || !bytes.Equal(chunk.Bytes, want) {
		t.Fatalf("chunk = %+v", chunk)
	}
	child.Input <- Input{}
	for chunk := range child.Output {
		t.Fatalf("unexpected output after EOF: %+v", chunk)
	}
	if err := unsuccessfulExit(child.Wait(t.Context())); err != nil {
		t.Fatal(err)
	}
	if err := unsuccessfulExit(child.Wait(t.Context())); err != nil {
		t.Fatal(err)
	}
	if err := unix.Kill(int(child.PID), 0); !errors.Is(err, unix.ESRCH) {
		t.Fatalf("child was not reaped: %v", err)
	}
}

func TestChildCancellationInterruptsBackpressure(t *testing.T) {
	child := spawnChild(t, childOptions(t, "/usr/bin/yes"))
	<-child.Output
	// Observe the bounded output queue actually fill, rather than sleeping and
	// assuming that the writer reached backpressure. The queue offers no event
	// for being full, so the loop yields between checks of its length.
	for len(child.Output) != cap(child.Output) {
		runtime.Gosched()
	}
	child.Cancel()
	exit, err := child.Wait(t.Context())
	if exit.Signal == nil || *exit.Signal != "SIGKILL" || err != nil {
		t.Fatalf("exit = %+v", exit)
	}
	if !child.IsCancelled() {
		t.Fatal("cancellation not recorded")
	}
}

func TestChildCancellationKillsDescendants(t *testing.T) {
	child := spawnChild(t, childOptions(t, "/bin/sh", "-c", "sleep 30 & printf '%s\\n' $!; wait"))
	output := <-child.Output
	pid, err := strconv.Atoi(strings.TrimSpace(string(output.Bytes)))
	if err != nil {
		t.Fatal(err)
	}
	child.Cancel()
	_, _ = child.Wait(t.Context())
	// A reparented zombie can remain until init reaps it, and nothing reports
	// that reaping to this process. Poll only the observed process state, never
	// a fixed settling interval.
	for {
		if err := unix.Kill(pid, 0); errors.Is(err, unix.ESRCH) {
			break
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
			_, err := Spawn(t.Context(), options)
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
	if err := wrapped.Start(t.Context()); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = wrapped.Kill()
		_, _ = wrapped.Wait(context.Background())
	})
	if err := unsuccessfulExit(wrapped.Wait(t.Context())); err != nil {
		t.Fatal(err)
	}
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
			err := command.Start(t.Context())
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
	if err := unsuccessfulExit(child.Wait(t.Context())); err != nil {
		t.Fatal(err)
	}
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
	if err := command.Start(t.Context()); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	exit, _ := command.Wait(ctx)
	if exit.Signal == nil || *exit.Signal != "SIGKILL" {
		t.Fatalf("exit %+v", exit)
	}
}

func TestCommandOutputFailureKillsChild(t *testing.T) {
	cmd := exec.Command("/usr/bin/yes")
	sentinel := fmt.Errorf("output failed")
	cmd.Stdout = failingOutput{err: sentinel}
	command := Wrap(cmd, true, ChildAttributes{})
	if err := command.Start(t.Context()); err != nil {
		t.Fatal(err)
	}
	exit, err := command.Wait(t.Context())
	if err == nil || !strings.Contains(err.Error(), sentinel.Error()) {
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
	if err := unsuccessfulExit(child.Wait(t.Context())); err != nil {
		t.Fatal(err)
	}
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
	// cat runs until the deferred close of its input, so a start that waited
	// for the running program would block here until go test -timeout.
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Fatalf("cancelled start: %v", err)
	}
}

func TestCommandCombinedOutputKeepsWriteOrder(t *testing.T) {
	var output bytes.Buffer
	cmd := exec.Command("/bin/sh", "-c", "printf first; printf second >&2; printf third")
	cmd.Stdout = &output
	cmd.Stderr = &output
	command := Wrap(cmd, true, ChildAttributes{})
	if err := command.Start(t.Context()); err != nil {
		t.Fatal(err)
	}
	if err := unsuccessfulExit(command.Wait(t.Context())); err != nil {
		t.Fatal(err)
	}
	if output.String() != "firstsecondthird" {
		t.Fatalf("combined output %q", output.String())
	}
}
