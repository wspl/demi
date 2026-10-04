//go:build darwin || linux

package process

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/cmdproto"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/cmdsdk/cmdsdktest"
	"golang.org/x/sys/unix"
)

type localHandler struct {
	invoke func(
		context.Context,
		cmdsdk.InvocationContext[cmdproto.LocalInvocation],
	) (cmdproto.Completion, error)
}

func (localHandler) Operations() []string {
	return []string{Raw}
}

func (h localHandler) Invoke(
	ctx context.Context,
	call cmdsdk.InvocationContext[cmdproto.LocalInvocation],
) (cmdproto.Completion, error) {
	return h.invoke(ctx, call)
}

type bufferOutput struct{ bytes.Buffer }

func (*bufferOutput) Close() error {
	return nil
}

func TestForwardPullDrivenBinaryAndCompletion(t *testing.T) {
	// Keep the Unix socket pathname below macOS's sockaddr_un limit.
	directory, err := os.MkdirTemp("", "demi-ipc-")
	if err != nil {
		t.Fatal(err)
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() {
		_ = os.RemoveAll(directory)
	}()
	endpoint := filepath.Join(directory, "socket")
	listener, err := net.Listen("unix", endpoint)
	if err != nil {
		t.Fatal(err)
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() {
		_ = listener.Close()
	}()
	input, writer := io.Pipe()
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() {
		_ = writer.Close()
	}()
	output := &bufferOutput{}
	errorOutput := &bufferOutput{}
	readRequested := make(chan struct{})
	completed := make(chan error, 1)
	ctx := t.Context()
	go func() {
		connection, err := listener.Accept()
		if err != nil {
			completed <- err
			return
		}
		completed <- cmdsdk.ServeLocal(ctx, connection, localHandler{invoke: func(
			ctx context.Context,
			call cmdsdk.InvocationContext[cmdproto.LocalInvocation],
		) (cmdproto.Completion, error) {
			if call.Request.Cwd != directory || call.Request.Env["VALUE"] != "<>&" {
				t.Errorf("metadata %+v", call.Request)
			}
			if err := call.Output.Stdout(ctx, []byte("before input")); err != nil {
				return cmdproto.Completion{}, err
			}
			close(readRequested)
			data, err := call.Input.Next(ctx)
			if err != nil {
				return cmdproto.Completion{}, err
			}
			if err := call.Output.Stdout(ctx, data); err != nil {
				return cmdproto.Completion{}, err
			}
			if err := call.Output.Stderr(ctx, []byte("error stream")); err != nil {
				return cmdproto.Completion{}, err
			}
			// Return without pulling EOF; forwarding must join its blocked input work.
			return cmdproto.Completion{ExitCode: 7}, nil
		}})
	}()
	sent := make(chan struct{})
	go func() {
		defer close(sent)
		<-readRequested
		_, _ = writer.Write([]byte{0, 255, 10})
	}()
	completion, err := Forward(
		ctx,
		endpoint,
		cmdproto.LocalInvocation{
			Operation:    Raw,
			InvocationID: "test",
			Args:         json.RawMessage(`{}`),
			Cwd:          directory,
			Env:          map[string]string{"VALUE": "<>&"},
		},
		Stdio{Stdin: input, Stdout: output, Stderr: errorOutput},
	)
	<-sent
	if err != nil {
		t.Fatal(err)
	}
	if completion.ExitCode != 7 {
		t.Fatalf("completion %+v", completion)
	}
	if !bytes.Equal(output.Bytes(), append([]byte("before input"), 0, 255, 10)) ||
		errorOutput.String() != "error stream" {
		t.Fatalf("stdout %q stderr %q", output.Bytes(), errorOutput.String())
	}
	// Forward closed the connection, so the server joins on EOF.
	if err := <-completed; err != nil && !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
}

func TestConnectWaitsForLiveRunnerAndFailsWhenGone(t *testing.T) {
	directory, err := os.MkdirTemp("", "demi-ipc-")
	if err != nil {
		t.Fatal(err)
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() {
		_ = os.RemoveAll(directory)
	}()
	endpoint := filepath.Join(directory, "socket")
	listener, err := net.ListenUnix("unix", &net.UnixAddr{Name: endpoint, Net: "unix"})
	if err != nil {
		t.Fatal(err)
	}
	listener.SetUnlinkOnClose(false)
	if err := listener.Close(); err != nil {
		t.Fatal(err)
	}
	lock, err := os.Create(filepath.Join(directory, Alive))
	if err != nil {
		t.Fatal(err)
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() {
		_ = lock.Close()
	}()
	if err := unix.Flock(int(lock.Fd()), unix.LOCK_EX|unix.LOCK_NB); err != nil {
		t.Fatal(err)
	}
	baseline := cmdsdktest.Pauses()
	done := make(chan error, 1)
	ctx := t.Context()
	go func() {
		connection, err := Connect(ctx, endpoint)
		if connection != nil {
			_ = connection.Close()
		}
		done <- err
	}()
	// The pause counter offers no event, so the loop yields between checks.
	for cmdsdktest.Pauses() == baseline {
		select {
		case err := <-done:
			t.Fatalf("live runner refused without waiting: %v", err)
		default:
		}
		runtime.Gosched()
	}
	if err := unix.Flock(int(lock.Fd()), unix.LOCK_UN); err != nil {
		t.Fatal(err)
	}
	if err := <-done; !errors.Is(err, unix.ECONNREFUSED) {
		t.Fatalf("gone runner: %v", err)
	}
	if _, err := Connect(ctx, "relative"); err == nil || !strings.Contains(err.Error(), "must be absolute") {
		t.Fatalf("relative endpoint: %v", err)
	}
}

func TestForwardOwnedFileInputIsInterruptible(t *testing.T) {
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		_ = writer.Close()
	}()
	// Fd switches the source pipe to blocking, as inherited standard IO is.
	duplicate, err := unix.FcntlInt(reader.Fd(), unix.F_DUPFD_CLOEXEC, 0)
	if err != nil {
		t.Fatal(err)
	}
	file := os.NewFile(uintptr(duplicate), "inherited stdin")
	flags, err := unix.FcntlInt(reader.Fd(), unix.F_GETFL, 0)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		_ = reader.Close()
	}()
	stdio, restore, err := cancellableStdio(
		t.Context(),
		Stdio{Stdin: file, Stdout: &bufferOutput{}, Stderr: &bufferOutput{}},
	)
	if err != nil {
		t.Fatal(err)
	}
	source := &commandReader{commandStream: commandStream[io.ReadCloser]{stream: stdio.Stdin}}
	ctx, cancel := context.WithCancel(t.Context())
	reading := make(chan struct{})
	done := make(chan error, 1)
	go func() {
		close(reading)
		_, err := source.Next(ctx)
		done <- err
	}()
	<-reading
	cancel()
	if err := <-done; err == nil {
		t.Fatal("blocked input was not cancelled")
	}
	if err := source.Close(); err != nil {
		t.Fatal(err)
	}
	if err := restore(); err != nil {
		t.Fatal(err)
	}
	restored, err := unix.FcntlInt(reader.Fd(), unix.F_GETFL, 0)
	if err != nil || restored != flags {
		t.Fatalf("shared flags: %x -> %x: %v", flags, restored, err)
	}
}

func TestForwardRestoresSharedOutputFlags(t *testing.T) {
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		_ = reader.Close()
	}()
	duplicate, err := unix.FcntlInt(writer.Fd(), unix.F_DUPFD_CLOEXEC, 0)
	if err != nil {
		t.Fatal(err)
	}
	kept := os.NewFile(uintptr(duplicate), "original shared output")
	defer func() {
		_ = kept.Close()
	}()
	flags, err := unix.FcntlInt(uintptr(duplicate), unix.F_GETFL, 0)
	if err != nil {
		t.Fatal(err)
	}
	stdio, restore, err := cancellableStdio(
		t.Context(),
		Stdio{Stdin: io.NopCloser(strings.NewReader("")), Stdout: writer, Stderr: writer},
	)
	if err != nil {
		t.Fatal(err)
	}
	if err := errors.Join(stdio.Stdin.Close(), stdio.Stdout.Close(), stdio.Stderr.Close(), restore()); err != nil {
		t.Fatal(err)
	}
	after, err := unix.FcntlInt(uintptr(duplicate), unix.F_GETFL, 0)
	if err != nil || after != flags {
		t.Fatalf("shared output flags: %x -> %x: %v", flags, after, err)
	}
}

// forwardOutput signals when forwarding reaches a pipe the caller never drains.
type forwardOutput struct {
	*io.PipeWriter
	started chan struct{}
}

func (w *forwardOutput) Write(b []byte) (int, error) {
	close(w.started)
	return w.PipeWriter.Write(b)
}

func TestForwardConnectionLossInterruptsBlockedOutput(t *testing.T) {
	// Uses one local connection and event waits; a forward that needed caller
	// cancellation blocks until go test -timeout. Both output streams must
	// release their blocked write.
	for _, stderr := range []bool{false, true} {
		name := "stdout"
		if stderr {
			name = "stderr"
		}
		t.Run(name, func(t *testing.T) {
			directory, err := os.MkdirTemp("", "demi-ipc-")
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				_ = os.RemoveAll(directory)
			}()
			endpoint := filepath.Join(directory, "socket")
			listener, err := net.Listen("unix", endpoint)
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				_ = listener.Close()
			}()
			reader, writer := io.Pipe()
			defer func() {
				_ = reader.Close()
			}()
			blocked := &forwardOutput{PipeWriter: writer, started: make(chan struct{})}
			stdio := Stdio{Stdin: io.NopCloser(strings.NewReader("")), Stdout: blocked, Stderr: &bufferOutput{}}
			if stderr {
				stdio.Stdout, stdio.Stderr = stdio.Stderr, stdio.Stdout
			}
			accepted := make(chan net.Conn, 1)
			served := make(chan error, 1)
			go func() {
				conn, err := listener.Accept()
				if err != nil {
					served <- err
					return
				}
				accepted <- conn
				served <- cmdsdk.ServeLocal(t.Context(), conn, localHandler{invoke: func(
					ctx context.Context,
					call cmdsdk.InvocationContext[cmdproto.LocalInvocation],
				) (cmdproto.Completion, error) {
					var err error
					if stderr {
						err = call.Output.Stderr(ctx, []byte("blocked"))
					} else {
						err = call.Output.Stdout(ctx, []byte("blocked"))
					}
					if err != nil {
						return cmdproto.Completion{}, err
					}
					<-ctx.Done()
					return cmdproto.Completion{}, ctx.Err()
				}})
			}()
			forwarded := make(chan error, 1)
			go func() {
				_, err := Forward(
					t.Context(),
					endpoint,
					cmdproto.LocalInvocation{
						Operation:    Raw,
						InvocationID: "lost",
						Args:         json.RawMessage(`{}`),
						Cwd:          directory,
						Env:          map[string]string{},
					},
					stdio,
				)
				forwarded <- err
			}()
			conn := <-accepted
			select {
			case <-blocked.started:
			case err := <-forwarded:
				<-served
				t.Fatalf("forwarding ended before output: %v", err)
			}
			if err := conn.Close(); err != nil {
				t.Error(err)
			}
			if err := <-forwarded; err == nil {
				t.Error("connection loss succeeded")
			}
			<-served
		})
	}
}
