package jobs_test

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"os"
	"runtime"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runner/process"
)

// localCommands plays the local caller's IO and cancellation scenarios.
type localCommands struct {
	invoke func(context.Context, cmdsdk.InvocationContext[commandwire.LocalInvocation]) (commandwire.Completion, error)
}

func (localCommands) Operations() []string { return []string{"fixture"} }

func (h localCommands) Invoke(
	ctx context.Context,
	inv cmdsdk.InvocationContext[commandwire.LocalInvocation],
) (commandwire.Completion, error) {
	return h.invoke(ctx, inv)
}

type outputBuffer struct{ bytes.Buffer }

func (*outputBuffer) Close() error { return nil }

type neverRead struct{ t testing.TB }

func (r neverRead) Read([]byte) (int, error) {
	r.t.Error("stdin consumed without demand")
	return 0, errors.New("stdin consumed without demand")
}
func (neverRead) Close() error { return nil }
func localRequest() commandwire.LocalInvocation {
	return commandwire.LocalInvocation{
		Operation:    "fixture",
		InvocationID: "test",
		Args:         json.RawMessage(`{}`),
		Cwd:          os.TempDir(),
		Env:          map[string]string{},
	}
}

func localServer(t *testing.T, h localCommands) *jobs.Server {
	t.Helper()
	server, err := jobs.StartServer(testContext(t), h)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := server.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return server
}

func TestCommandWithoutStdinNeverPollsSource(t *testing.T) {
	server := localServer(
		t,
		localCommands{
			invoke: func(
				ctx context.Context,
				inv cmdsdk.InvocationContext[commandwire.LocalInvocation],
			) (commandwire.Completion, error) {
				return commandwire.Completion{}, inv.Output.Stdout(ctx, []byte("done\x00\xff"))
			},
		},
	)
	stdout := &outputBuffer{}
	result, err := process.Forward(
		testContext(t),
		server.Endpoint(),
		localRequest(),
		process.Stdio{Stdin: neverRead{t: t}, Stdout: stdout, Stderr: &outputBuffer{}},
	)
	if err != nil || result.ExitCode != 0 || !bytes.Equal(stdout.Bytes(), []byte("done\x00\xff")) {
		t.Fatalf("completion %+v, error %v, output %q", result, err, stdout.Bytes())
	}
}

func TestPendingTerminalInputAllowsOutputAndCompletion(t *testing.T) {
	requested := make(chan struct{})
	server := localServer(
		t,
		localCommands{
			invoke: func(
				ctx context.Context,
				inv cmdsdk.InvocationContext[commandwire.LocalInvocation],
			) (commandwire.Completion, error) {
				inputCtx, cancel := context.WithCancel(ctx)
				done := make(chan struct{})
				go func() {
					defer close(done)
					_, _ = inv.Input.Next(inputCtx)
				}()
				defer func() {
					cancel()
					<-done
				}()
				select {
				case <-ctx.Done():
					return commandwire.Completion{}, ctx.Err()
				case <-requested:
				}
				return commandwire.Completion{}, inv.Output.Stdout(ctx, []byte("done\x00\xff"))
			},
		},
	)
	reader, writer := io.Pipe()
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() { _ = writer.Close() }()
	stdout := &outputBuffer{}
	input := &observedRead{ReadCloser: reader, started: requested}
	result, err := process.Forward(
		testContext(t),
		server.Endpoint(),
		localRequest(),
		process.Stdio{Stdin: input, Stdout: stdout, Stderr: &outputBuffer{}},
	)
	if err != nil || result.ExitCode != 0 || stdout.String() != "done\x00\xff" {
		t.Fatalf("result %+v, %v, output %q", result, err, stdout.String())
	}
}

type observedRead struct {
	io.ReadCloser
	started chan struct{}
	once    sync.Once
}

func (r *observedRead) Read(bytes []byte) (int, error) {
	r.once.Do(func() { close(r.started) })
	return r.ReadCloser.Read(bytes)
}

func TestCancellationInterruptsBlockedOutput(t *testing.T)   { blockedOutput(t, false) }
func TestConnectionLossInterruptsBlockedStdout(t *testing.T) { blockedOutput(t, true) }
func blockedOutput(t *testing.T, disconnect bool) {
	t.Helper()
	server := localServer(
		t,
		localCommands{
			invoke: func(
				ctx context.Context,
				inv cmdsdk.InvocationContext[commandwire.LocalInvocation],
			) (commandwire.Completion, error) {
				for {
					if err := inv.Output.Stdout(ctx, make([]byte, 65536)); err != nil {
						return commandwire.Completion{}, err
					}
				}
			},
		},
	)
	reader, writer := io.Pipe()
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() { _ = reader.Close() }()
	ctx, cancel := context.WithCancel(testContext(t))
	defer cancel()
	done := make(chan error, 1)
	go func() {
		_, err := process.Forward(
			ctx,
			server.Endpoint(),
			localRequest(),
			process.Stdio{Stdin: neverRead{t: t}, Stdout: writer, Stderr: &outputBuffer{}},
		)
		done <- err
	}()
	buffer := make([]byte, 1)
	if _, err := io.ReadFull(reader, buffer); err != nil {
		t.Fatal(err)
	}
	if disconnect {
		if err := server.Close(testContext(t)); err != nil {
			t.Fatal(err)
		}
	} else {
		cancel()
	}
	if err := <-done; err == nil || (!disconnect && !errors.Is(err, context.Canceled)) {
		t.Fatalf("caller error %v", err)
	}
	if disconnect && ctx.Err() != nil {
		t.Fatal("transport loss did not interrupt blocked stdout before the hang deadline")
	}
}

func TestPrivateEndpointStreamsBinaryInputAndJoinsCancellation(t *testing.T) {
	cancelled := make(chan struct{})
	server := localServer(
		t,
		localCommands{
			invoke: func(
				ctx context.Context,
				inv cmdsdk.InvocationContext[commandwire.LocalInvocation],
			) (commandwire.Completion, error) {
				if inv.Request.InvocationID == "wait" {
					if err := inv.Output.Stdout(ctx, []byte("ready")); err != nil {
						return commandwire.Completion{}, err
					}
					<-ctx.Done()
					close(cancelled)
					return commandwire.Completion{}, ctx.Err()
				}
				for {
					bytes, err := inv.Input.Next(ctx)
					if errors.Is(err, io.EOF) {
						return commandwire.Completion{}, nil
					}
					if err != nil {
						return commandwire.Completion{}, err
					}
					if err = inv.Output.Stdout(ctx, bytes); err != nil {
						return commandwire.Completion{}, err
					}
				}
			},
		},
	)
	if runtime.GOOS != "windows" {
		stat, err := os.Stat(server.Endpoint())
		if err != nil || stat.Mode().Perm() != 0o600 {
			t.Fatalf("private socket: %v %v", stat, err)
		}
	}
	ctx := testContext(t)
	conn, err := process.Connect(ctx, server.Endpoint())
	if err != nil {
		t.Fatal(err)
	}
	client, err := cmdsdk.Connect(ctx, conn)
	if err != nil {
		t.Fatal(err)
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() { _ = client.Close() }()
	input, output, err := client.Invoke(ctx, localRequest())
	if err != nil {
		t.Fatal(err)
	}
	defer input.Cancel()
	record, err := output.Next(ctx)
	if _, ok := record.(commandwire.InputPull); err != nil || !ok {
		t.Fatalf("demand: %T %v", record, err)
	}
	if err = input.Write(ctx, []byte("raw\x00\xff")); err != nil {
		t.Fatal(err)
	}
	record, err = output.Next(ctx)
	if out, ok := record.(commandwire.Stdout); err != nil || !ok || !bytes.Equal(out, []byte("raw\x00\xff")) {
		t.Fatalf("binary output: %v %v", record, err)
	}
	record, err = output.Next(ctx)
	if _, ok := record.(commandwire.InputPull); err != nil || !ok {
		t.Fatalf("EOF demand: %T %v", record, err)
	}
	if err = input.End(); err != nil {
		t.Fatal(err)
	}
	record, err = output.Next(ctx)
	if _, ok := record.(commandwire.Completed); err != nil || !ok {
		t.Fatalf("completion: %T %v", record, err)
	}
	if _, err = output.Next(ctx); !errors.Is(err, io.EOF) {
		t.Fatalf("terminal: %v", err)
	}
	request := localRequest()
	request.InvocationID = "wait"
	input, output, err = client.Invoke(ctx, request)
	if err != nil {
		t.Fatal(err)
	}
	defer input.Cancel()
	record, err = output.Next(ctx)
	if bytes, ok := record.(commandwire.Stdout); err != nil || !ok || string(bytes) != "ready" {
		t.Fatalf("ready: %v %v", record, err)
	}
	if err = server.Close(ctx); err != nil {
		t.Fatal(err)
	}
	select {
	case <-cancelled:
	default:
		t.Fatal("server returned without joining invocation")
	}
	if runtime.GOOS != "windows" {
		if _, err := os.Stat(server.Endpoint()); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("endpoint retained: %v", err)
		}
	}
}
