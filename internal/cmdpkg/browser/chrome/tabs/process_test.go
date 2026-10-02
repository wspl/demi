//go:build unix

package tabs

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"syscall"
	"testing"
	"time"
)

// TestDetachedHelperFixture is a child-process fixture, not a timed wait: its
// parent owns stdin and kills or releases it after observing the ready line.
func TestDetachedHelperFixture(t *testing.T) {
	if os.Getenv("DEMI_BROWSER_HELPER_FIXTURE") != "1" {
		return
	}
	if _, err := fmt.Fprintln(os.Stdout, "ready"); err != nil {
		t.Fatal(err)
	}
	if _, err := io.Copy(io.Discard, os.Stdin); err != nil {
		t.Fatal(err)
	}
}

// Process retirement must find a marked helper that escaped Chrome's process
// group, but must not inspect/kill an unrelated executable with the same marker.
// The child readiness pipe replaces the Rust fixture's 60-second sleep.
func TestRetirementIncludesMarkedHelpersInAnotherSessionOnly(t *testing.T) {
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	executable, err = filepath.EvalSymlinks(executable)
	if err != nil {
		t.Fatal(err)
	}
	profile := t.TempDir()
	owner := &chromeProcess{runtime: profile, installations: []string{filepath.Dir(executable)}}
	child := func() (*exec.Cmd, io.WriteCloser, io.ReadCloser) {
		command := exec.Command(executable, "-test.run=^TestDetachedHelperFixture$", "-test.count=1")
		command.Env = append(os.Environ(), "DEMI_BROWSER_HELPER_FIXTURE=1", "DEMI_BROWSER_PROFILE="+profile)
		input, err := command.StdinPipe()
		if err != nil {
			t.Fatal(err)
		}
		output, err := command.StdoutPipe()
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() {
			_ = input.Close()
			_ = output.Close()
		})
		return command, input, output
	}
	leader, _, leaderOutput := child()
	if err := owner.start(leader, profile); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if err := owner.retire(ctx); err != nil {
			t.Error(err)
		}
	})
	if line, err := bufio.NewReader(leaderOutput).ReadString('\n'); err != nil || line != "ready\n" {
		t.Fatalf("leader readiness %q: %v", line, err)
	}
	helper, _, output := child()
	helper.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	if err := helper.Start(); err != nil {
		t.Fatal(err)
	}
	helperDone := make(chan error, 1)
	go func() { helperDone <- helper.Wait() }()
	t.Cleanup(func() {
		_ = helper.Process.Kill()
		<-helperDone
	})
	if line, err := bufio.NewReader(output).ReadString('\n'); err != nil || line != "ready\n" {
		t.Fatalf("helper readiness %q: %v", line, err)
	}
	unrelated := exec.Command("/bin/cat")
	unrelated.Env = append(os.Environ(), "DEMI_BROWSER_PROFILE="+profile)
	unrelatedInput, err := unrelated.StdinPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := unrelated.Start(); err != nil {
		t.Fatal(err)
	}
	unrelatedDone := make(chan error, 1)
	go func() { unrelatedDone <- unrelated.Wait() }()
	t.Cleanup(func() {
		_ = unrelatedInput.Close()
		_ = unrelated.Process.Kill()
		<-unrelatedDone
	})
	marked, err := markedProcesses(owner.installations, "DEMI_BROWSER_PROFILE="+profile)
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Contains(marked, helper.Process.Pid) {
		t.Fatalf("detached helper %d not observed: %v", helper.Process.Pid, marked)
	}
	ctx, cancel := context.WithTimeout(t.Context(), 5*time.Second)
	defer cancel()
	if err := owner.retire(ctx); err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-helperDone:
		helperDone <- err // Cleanup owns the final join on every path.
		var exit *exec.ExitError
		if !errors.As(err, &exit) || !exit.Sys().(syscall.WaitStatus).Signaled() {
			t.Fatalf("helper not killed: %v", err)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	select {
	case err := <-unrelatedDone:
		unrelatedDone <- err
		t.Fatalf("unrelated process ended: %v", err)
	default:
	}
}
