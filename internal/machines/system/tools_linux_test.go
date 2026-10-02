//go:build linux

package system_test

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"
	"unicode/utf8"

	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machines/system/systemtest"
	"golang.org/x/sys/unix"
)

// TestToolProcess is a local child fixture; no external service is contacted.
func TestToolProcess(_ *testing.T) {
	if os.Getenv("DEMI_SYSTEM_TOOL_CHILD") != "1" {
		return
	}
	args := os.Args
	for len(args) > 0 && args[0] != "--" {
		args = args[1:]
	}
	if len(args) < 2 {
		os.Exit(90)
	}
	switch args[1] {
	case "output":
		data, err := io.ReadAll(os.Stdin)
		if err != nil || len(data) != 0 {
			os.Exit(91)
		}
		if _, err := fmt.Fprint(os.Stdout, os.Getenv("LC_ALL")); err != nil {
			os.Exit(96)
		}
		if _, err := os.Stderr.Write([]byte{' ', 0xff, 0xff, 0xe2, 0x82, ' ', '\n'}); err != nil {
			os.Exit(96)
		}
		os.Exit(7)
	case "wait":
		ready, err := os.OpenFile(args[2], os.O_WRONLY, 0)
		if err != nil {
			os.Exit(92)
		}
		if _, err := fmt.Fprintf(ready, "%d\n", os.Getpid()); err != nil {
			os.Exit(96)
		}
		if err := ready.Close(); err != nil {
			os.Exit(96)
		}
		// Wait for signals; the parent cancels and reaps this child.
		for {
			_ = unix.Pause()
		}

	case "signal":
		if err := unix.Kill(os.Getpid(), unix.SIGKILL); err != nil {
			os.Exit(96)
		}
	case "fault":
		system.FaultPoint("fixture")
		os.Exit(0)
	}
	os.Exit(95)
}

// childTools resolves this test executable as an infrastructure-tool stand-in.
func childTools(t *testing.T) *system.Tools {
	t.Helper()
	t.Setenv("DEMI_SYSTEM_TOOL_CHILD", "1")
	// A coverage-instrumented child must not add runtime warnings to its output.
	t.Setenv("GOCOVERDIR", t.TempDir())
	path, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	return system.NewTools(map[system.Tool]string{system.Runsc: path})
}

func childArgs(args ...string) []string {
	return append([]string{"-test.run=^TestToolProcess$", "--"}, args...)
}

func TestToolOutputAndFailures(t *testing.T) {
	tools := childTools(t)
	t.Setenv("LC_ALL", "inherited-locale")
	out, err := tools.Output(t.Context(), system.Runsc, childArgs("output"), nil)
	if err != nil {
		t.Fatal(err)
	}
	if !out.Status.Exited() || out.Status.ExitStatus() != 7 || out.Stdout != "C.UTF-8" || out.Stderr != " \ufffd\ufffd\ufffd \n" {
		t.Fatalf("output = %#v", out)
	}
	if _, err := system.Accept(system.Runsc, out, []int{0, 7}); err != nil {
		t.Fatal(err)
	}
	_, err = tools.Run(t.Context(), system.Runsc, childArgs("output"), nil)
	var failed *system.FailedError
	if !errors.As(err, &failed) || failed.Error() != "runsc exited 7: \ufffd\ufffd\ufffd" {
		t.Fatalf("failed tool = %v", err)
	}
	_, err = tools.Run(t.Context(), system.Runsc, childArgs("signal"), nil)
	if !errors.As(err, &failed) || !failed.Output.Status.Signaled() || failed.Error() != "runsc exited by signal: " {
		t.Fatalf("signaled tool = %v", err)
	}
	missing := system.NewTools(map[system.Tool]string{system.Runsc: filepath.Join(t.TempDir(), "missing")})
	_, err = missing.Run(t.Context(), system.Runsc, nil, nil)
	var spawn *system.SpawnError
	if !errors.As(err, &spawn) || !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("spawn error lost cause: %v", err)
	}
}

func TestToolCancellationReapsChild(t *testing.T) {
	tools := childTools(t)
	fifo := filepath.Join(t.TempDir(), "ready")
	if err := unix.Mkfifo(fifo, 0600); err != nil {
		t.Fatal(err)
	}
	ready, err := os.OpenFile(fifo, os.O_RDWR, 0)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := ready.Close(); err != nil {
			t.Error(err)
		}
	}()
	if err := ready.SetReadDeadline(time.Now().Add(10 * time.Second)); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan struct{})
	var result error
	go func() {
		defer close(done)
		_, result = tools.Output(ctx, system.Runsc, childArgs("wait", fifo), nil)
	}()
	// Cancel and join even when readiness fails.
	defer func() {
		cancel()
		<-done
	}()
	line, err := bufio.NewReader(ready).ReadString('\n')
	if err != nil {
		t.Fatal(err)
	}
	var pid int
	if _, err := fmt.Sscanf(line, "%d", &pid); err != nil {
		t.Fatal(err)
	}
	// Retain the child's stdout as a descendant would. Cancellation must close
	// our read worker even while this writer remains open.
	retained, err := os.OpenFile(fmt.Sprintf("/proc/%d/fd/1", pid), os.O_WRONLY, 0)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := retained.Close(); err != nil {
			t.Error(err)
		}
	}()
	cancel()
	guard := time.NewTimer(5 * time.Second)
	defer guard.Stop()
	select {
	case <-done:
	case <-guard.C:
		t.Fatal("cancellation waited for an inherited output descriptor")
	}
	if !errors.Is(result, context.Canceled) {
		t.Fatalf("canceled tool = %v", result)
	}
	if err := unix.Kill(pid, 0); !errors.Is(err, unix.ESRCH) {
		t.Fatalf("child %d remains after return: %v", pid, err)
	}
}

func TestToolDeadline(t *testing.T) {
	tools := childTools(t)
	fifo := filepath.Join(t.TempDir(), "unopened")
	if err := unix.Mkfifo(fifo, 0600); err != nil {
		t.Fatal(err)
	}
	zero := time.Duration(0)
	_, err := tools.Output(t.Context(), system.Runsc, childArgs("wait", fifo), &zero)
	var deadline *system.DeadlineError
	if !errors.As(err, &deadline) || deadline.Error() != "runsc did not finish within 0 s" {
		t.Fatalf("deadline = %v", err)
	}
}

func TestToolMessageTail(t *testing.T) {
	for _, tc := range []struct {
		name   string
		output system.Output
		want   string
	}{
		{"stderr", system.Output{Stdout: "ignore", Stderr: " \nproblem \t"}, "problem"},
		{"stdout", system.Output{Stdout: " fallback \n", Stderr: " \n"}, "fallback"},
		{"utf8 boundary", system.Output{Stderr: "x" + strings.Repeat("界", 2731)}, strings.Repeat("界", 2730)},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got := tc.output.Message()
			if got != tc.want || !utf8.ValidString(got) {
				t.Fatalf("message length %d, want %d", len(got), len(tc.want))
			}
		})
	}
	if got := system.Tail("界", 2); got != "" {
		t.Fatalf("partial rune = %q", got)
	}
	if got := system.Tail("abc", 0); got != "" {
		t.Fatalf("zero tail = %q", got)
	}
}

func TestResolveAndTestTools(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("PATH", dir)
	runsc := filepath.Join(dir, "configured-runsc")
	if err := os.WriteFile(runsc, nil, 0600); err != nil {
		t.Fatal(err)
	}
	_, err := system.Resolve(t.Context(), runsc)
	var missing *system.MissingTools
	if !errors.As(err, &missing) || err.Error() != "Cloud manager needs: mke2fs, e2fsck, resize2fs, bsdtar, nft" {
		t.Fatalf("missing tools = %v", err)
	}
	for _, name := range []string{"mke2fs", "e2fsck", "resize2fs", "bsdtar", "nft"} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte("#!/bin/sh\nexit 0\n"), 0700); err != nil {
			t.Fatal(err)
		}
	}
	tools, err := system.Resolve(t.Context(), runsc)
	if err != nil {
		t.Fatal(err)
	}
	if tools.Path(system.Runsc) != runsc {
		t.Fatalf("configured runsc = %s", tools.Path(system.Runsc))
	}
	if _, err := tools.Run(t.Context(), system.Mke2fs, nil, nil); err != nil {
		t.Fatal(err)
	}
	fixture, err := systemtest.OnPath(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if fixture.Path(system.Runsc) != "runsc" {
		t.Fatalf("fixture runsc = %s", fixture.Path(system.Runsc))
	}
	command := systemtest.Placeholder().Command(t.Context(), system.Runsc)
	command.Args = append(command.Args, "--root", "fixture")
	if strings.Join(command.Args, " ") != " --root fixture" {
		t.Fatalf("placeholder command = %q", command.Args)
	}
	// Relative PATH entries are accepted by Rust's which and become explicit
	// absolute paths before os/exec applies its ErrDot protection.
	t.Chdir(dir)
	t.Setenv("PATH", ".")
	relative, err := system.Resolve(t.Context(), runsc)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := relative.Run(t.Context(), system.Mke2fs, nil, nil); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(runsc); err != nil {
		t.Fatal(err)
	}
	_, err = system.Resolve(t.Context(), runsc)
	if !errors.As(err, &missing) || err.Error() != "Cloud manager needs: runsc" {
		t.Fatalf("missing runsc = %v", err)
	}
}

func TestSystemFailurePreservesErrno(t *testing.T) {
	err := system.Failed("mounting", "/fixture", syscall.EPERM)
	if !errors.Is(err, syscall.EPERM) || err.Error() != "mounting /fixture: operation not permitted" {
		t.Fatalf("failed = %v", err)
	}
}
