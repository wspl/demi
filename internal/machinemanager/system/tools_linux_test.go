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

	"github.com/wspl/demi/internal/machinemanager/system"
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
	out, err := tools.Output(t.Context(), system.Runsc, childArgs("output"), 0)
	if err != nil {
		t.Fatal(err)
	}
	if !out.Status.Exited() || out.Status.ExitStatus() != 7 || out.Stdout != "C.UTF-8" ||
		out.Stderr != " \ufffd\ufffd\ufffd \n" {
		t.Fatalf("output = %#v", out)
	}
	if _, err := system.Accept(system.Runsc, out, []int{0, 7}); err != nil {
		t.Fatal(err)
	}
	_, err = tools.Run(t.Context(), system.Runsc, childArgs("output"), 0)
	if err == nil || err.Error() != "runsc exited 7: ���" {
		t.Fatalf("failed tool = %v", err)
	}
	signaled, err := tools.Output(t.Context(), system.Runsc, childArgs("signal"), 0)
	if err != nil || !signaled.Status.Signaled() {
		t.Fatalf("signaled tool = %+v, %v", signaled, err)
	}
	if _, err = system.Accept(
		system.Runsc,
		signaled,
		[]int{0},
	); err == nil ||
		err.Error() != "runsc exited by signal: " {
		t.Fatalf("signaled tool = %v", err)
	}
	missing := system.NewTools(map[system.Tool]string{system.Runsc: filepath.Join(t.TempDir(), "missing")})
	_, err = missing.Run(t.Context(), system.Runsc, nil, 0)
	if err == nil || !strings.HasPrefix(err.Error(), "runsc could not start: ") || !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("spawn error lost cause: %v", err)
	}
}

func TestToolCancellationReapsChild(t *testing.T) {
	tools := childTools(t)
	fifo := filepath.Join(t.TempDir(), "ready")
	if err := unix.Mkfifo(fifo, 0o600); err != nil {
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
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan struct{})
	var result error
	go func() {
		defer close(done)
		_, result = tools.Output(ctx, system.Runsc, childArgs("wait", fifo), 0)
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
	// Output must return while the retained writer is still open; if it waited
	// for that descriptor, this receive would hang until go test's timeout.
	<-done
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
	if err := unix.Mkfifo(fifo, 0o600); err != nil {
		t.Fatal(err)
	}
	_, err := tools.Output(t.Context(), system.Runsc, childArgs("wait", fifo), time.Nanosecond)
	if err == nil || err.Error() != "runsc did not finish within 0 s" {
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

func TestResolveTools(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("PATH", dir)
	runsc := filepath.Join(dir, "configured-runsc")
	if err := os.WriteFile(runsc, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	_, err := system.Resolve(t.Context(), runsc)
	if err == nil || err.Error() != "Cloud manager needs: mke2fs, e2fsck, resize2fs, bsdtar" {
		t.Fatalf("missing tools = %v", err)
	}
	for _, name := range []string{"mke2fs", "e2fsck", "resize2fs", "bsdtar"} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte("#!/bin/sh\nexit 0\n"), 0o700); err != nil {
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
	if _, err := tools.Run(t.Context(), system.Mke2fs, nil, 0); err != nil {
		t.Fatal(err)
	}
	// Relative PATH entries are accepted and become explicit absolute paths
	// before os/exec applies its ErrDot protection.
	t.Chdir(dir)
	t.Setenv("PATH", ".")
	relative, err := system.Resolve(t.Context(), runsc)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := relative.Run(t.Context(), system.Mke2fs, nil, 0); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(runsc); err != nil {
		t.Fatal(err)
	}
	_, err = system.Resolve(t.Context(), runsc)
	if err == nil || err.Error() != "Cloud manager needs: runsc" {
		t.Fatalf("missing runsc = %v", err)
	}
}

func TestSystemFailurePreservesErrno(t *testing.T) {
	err := system.Failed("mounting", "/fixture", syscall.EPERM)
	if !errors.Is(err, syscall.EPERM) || err.Error() != "mounting /fixture: operation not permitted" {
		t.Fatalf("failed = %v", err)
	}
}
