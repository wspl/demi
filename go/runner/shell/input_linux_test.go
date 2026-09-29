//go:build linux

package shell_test

import (
	"context"
	"os"
	"testing"
	"time"

	sys "golang.org/x/sys/unix"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
	"github.com/wspl/demi/go/runner/shell"
)

// A declared command's read of its input ends when its call is cancelled,
// also after an external command switched the shared pipe to blocking mode,
// where closing the descriptor wakes no read.
// Cost: one external command and a loopback service; a five-second guard
// bounds every wait.
func TestCancellationEndsABlockedInputRead(t *testing.T) {
	opts := options(t)
	opts.Commands = commands(t, declaration)
	server := servicetest.Start(t, service{func(*commandservice.Call) (commandservice.Completion, error) {
		return commandservice.Completion{}, nil
	}})
	opts.Commands.Services = &services{client: server.Client}
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer reader.Close()
	defer writer.Close()
	opts.Stdin = reader
	guard, stop := context.WithTimeout(t.Context(), 5*time.Second)
	defer stop()
	ctx, cancel := context.WithCancel(guard)
	defer cancel()
	done := make(chan shell.Result, 1)
	go func() { done <- shell.Run(ctx, "/bin/true; demi", opts) }()
	if _, err := writer.Write([]byte("partial")); err != nil {
		t.Fatal(err)
	}
	drained(t, guard, reader)
	cancel()
	select {
	case <-done:
	case <-guard.Done():
		t.Fatal("the input read did not end")
	}
}

// drained waits until the declared command has read what the pipe held, the
// sign that its read has begun. No event marks a read, so it polls the pipe's
// count of unread bytes (FIONREAD, which Linux names TIOCINQ).
func drained(t *testing.T, ctx context.Context, pipe *os.File) {
	t.Helper()
	raw, err := pipe.SyscallConn()
	if err != nil {
		t.Fatal(err)
	}
	for {
		var pending int
		var countErr error
		if err := raw.Control(func(fd uintptr) { pending, countErr = sys.IoctlGetInt(int(fd), sys.TIOCINQ) }); err != nil || countErr != nil {
			t.Fatal(err, countErr)
		}
		if pending == 0 {
			return
		}
		select {
		case <-ctx.Done():
			t.Fatal("the declared command did not read its input")
		case <-time.After(time.Millisecond):
		}
	}
}
