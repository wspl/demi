//go:build unix

package process

import (
	"bufio"
	"context"
	"errors"
	"io"
	"os"
	"os/exec"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"testing"
	"time"
)

// One shell and a builtin-only descendant per case. Readiness, exit, and EOF
// are the synchronization events; ten minutes only bounds a broken test.
func TestContainedProcessAndDescendant(t *testing.T) {
	for _, cancelCommand := range []bool{false, true} {
		t.Run(strconv.FormatBool(cancelCommand), func(t *testing.T) {
			ctx, cancel := context.WithTimeout(t.Context(), 10*time.Minute)
			defer cancel()
			input, inputWriter, err := os.Pipe()
			if err != nil {
				t.Fatal(err)
			}
			defer input.Close()
			defer inputWriter.Close()
			output, outputWriter, err := os.Pipe()
			if err != nil {
				t.Fatal(err)
			}
			defer output.Close()
			defer outputWriter.Close()
			cmd := exec.CommandContext(ctx, "/bin/sh", "-c", `trap 'exit 9' TERM; (read hold <&3) & printf '%s\n' "$!"; wait`)
			cmd.ExtraFiles = []*os.File{input}
			cmd.Stdout = outputWriter
			group, err := Contain(cmd)
			if err != nil {
				t.Fatal(err)
			}
			defer group.Close()
			if err := cmd.Start(); err != nil {
				t.Fatal(err)
			}
			defer func() {
				_ = cmd.Process.Kill()
				_ = cmd.Wait() // Reap on assertion failure; a second Wait is harmless.
			}()
			outputWriter.Close()
			// Cancellation also releases a broken test's read, independently of Kill.
			stop := context.AfterFunc(ctx, func() { output.Close() })
			defer stop()
			reader := bufio.NewReader(output)
			line, err := reader.ReadString('\n')
			if err != nil {
				t.Fatal(err)
			}
			descendant, err := strconv.Atoi(strings.TrimSpace(line))
			if err != nil {
				t.Fatal(err)
			}
			defer syscall.Kill(descendant, syscall.SIGKILL)
			for _, pid := range []int{cmd.Process.Pid, descendant} {
				pgid, err := syscall.Getpgid(pid)
				if err != nil || pgid != cmd.Process.Pid {
					t.Fatalf("pid=%d group=%d want=%d err=%v", pid, pgid, cmd.Process.Pid, err)
				}
			}
			if cancelCommand {
				cancel()
			} else if err := group.Kill(); err != nil {
				t.Fatal(err)
			}
			err = cmd.Wait()
			var exited *exec.ExitError
			if !errors.As(err, &exited) || exited.Sys().(syscall.WaitStatus).Signal() != syscall.SIGKILL {
				t.Fatalf("kill status: %v", err)
			}
			if !cancelCommand {
				if _, err := reader.ReadByte(); err != io.EOF {
					t.Fatalf("descendant retained stdout: %v", err)
				}
			}
			if err := group.Close(); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestGoneErrors(t *testing.T) {
	for _, test := range []struct {
		err  error
		want bool
	}{
		{nil, false}, {syscall.ESRCH, true}, {syscall.EPERM, runtime.GOOS == "darwin"}, {syscall.EACCES, false},
	} {
		if got := gone(test.err); got != test.want {
			t.Fatalf("gone(%v)=%v want %v", test.err, got, test.want)
		}
	}
	cmd := exec.Command("/missing/program")
	group, err := Contain(cmd)
	if err != nil {
		t.Fatal(err)
	}
	if err := cmd.Start(); err == nil {
		t.Fatal("missing program started")
	}
	if err := group.Close(); err != nil {
		t.Fatal(err)
	}
}
