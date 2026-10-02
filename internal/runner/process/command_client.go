package process

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"sync"
	"time"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// EndpointEnv names the runner's local command endpoint.
const EndpointEnv = "DEMI_RUNNER_ENDPOINT"

// ContextEnv names the job's execution context.
const ContextEnv = "DEMI_CONTEXT_ID"

// Raw is the local operation whose arguments are a RawCommand.
const Raw = "raw"

// Alive is the file beside a Unix socket locked while its runner runs.
const Alive = "ipc.alive"

// Stdio supplies the caller's input and output. Forward takes ownership and
// closes all three on completion or cancellation; Close must unblock IO.
// Use StandardFile to pass duplicates of process standard handles.
type Stdio struct {
	Stdin  io.ReadCloser
	Stdout io.WriteCloser
	Stderr io.WriteCloser
}

// Forward forwards one local invocation. Input is read only after an explicit
// pull, while output and connection processing continue independently.
func Forward(ctx context.Context, endpoint string, request commandwire.LocalInvocation, stdio Stdio) (completion commandwire.Completion, err error) {
	defer func() {
		if err != nil && ctx.Err() != nil {
			err = &operationFailure{message: "command cancelled", cause: ctx.Err()}
		}
	}()
	stdio, restore, err := cancellableStdio(ctx, stdio)
	source := &commandReader{commandStream: commandStream[io.ReadCloser]{stream: stdio.Stdin}}
	terminal := &commandTerminal{stdout: commandStream[io.WriteCloser]{stream: stdio.Stdout}, stderr: commandStream[io.WriteCloser]{stream: stdio.Stderr}}
	defer func() {
		err = errors.Join(err, source.Close(), terminal.stdout.Close(), terminal.stderr.Close(), restore())
	}()
	if err != nil {
		return completion, err
	}
	connection, err := Connect(ctx, endpoint)
	if err != nil {
		return completion, err
	}
	invocation, cancel := context.WithCancel(ctx)
	defer cancel()
	client, err := cmdsdk.Connect(invocation, &commandConnection{Conn: connection, cancel: cancel})
	if err != nil {
		return completion, err
	}
	defer func() { err = errors.Join(err, client.Close()) }()
	// Only invocation opening has the Rust client's ten-second deadline. Its
	// stream retains the parent context after headers arrive.
	timeoutDone := make(chan struct{})
	timer := time.AfterFunc(10*time.Second, func() { cancel(); close(timeoutDone) })
	input, output, err := client.Invoke(invocation, request)
	if !timer.Stop() {
		<-timeoutDone
	}
	if err != nil {
		return completion, fmt.Errorf("local invocation: %w", err)
	}
	completion, err = (cmdsdk.Exchange{Input: input, Output: output}).Run(invocation, source, terminal)
	if err != nil {
		return completion, err
	}
	for _, stream := range []io.WriteCloser{stdio.Stdout, stdio.Stderr} {
		if flush, ok := stream.(interface{ Flush() error }); ok {
			if err := flush.Flush(); err != nil {
				return completion, err
			}
		}
	}
	return completion, nil
}

// commandConnection cancels local forwarding when the HTTP/2 transport fails.
// The HTTP/2 reader continues while Exchange is blocked writing caller output;
// cancelling its context interrupts that write without another IO worker.
type commandConnection struct {
	net.Conn
	cancel context.CancelFunc
}

func (c *commandConnection) Read(b []byte) (int, error) {
	n, err := c.Conn.Read(b)
	if err != nil {
		c.cancel()
	}
	return n, err
}

func (c *commandConnection) Write(b []byte) (int, error) {
	n, err := c.Conn.Write(b)
	if err != nil {
		c.cancel()
	}
	return n, err
}

// commandStream gives each owned invocation stream exactly one close, including
// when an exchange cancellation interrupts IO before Forward returns.
type commandStream[T io.Closer] struct {
	stream T
	once   sync.Once
	err    error
}

func (s *commandStream[T]) Close() error {
	s.once.Do(func() { s.err = s.stream.Close() })
	return s.err
}

type commandReader struct{ commandStream[io.ReadCloser] }

func (r *commandReader) Next(ctx context.Context) ([]byte, error) {
	stop := interruptCommandIO(ctx, r)
	defer stop()
	buffer := make([]byte, commandwire.MaxRecordBytes)
	n, err := r.stream.Read(buffer)
	if n > 0 {
		return buffer[:n], nil
	}
	if err == nil {
		err = io.EOF
	}
	return nil, err
}

type commandTerminal struct{ stdout, stderr commandStream[io.WriteCloser] }

func (t *commandTerminal) Stdout(ctx context.Context, b []byte) error {
	return t.write(ctx, &t.stdout, b)
}
func (t *commandTerminal) Stderr(ctx context.Context, b []byte) error {
	return t.write(ctx, &t.stderr, b)
}
func (*commandTerminal) write(ctx context.Context, target *commandStream[io.WriteCloser], b []byte) error {
	stop := interruptCommandIO(ctx, target)
	defer stop()
	for len(b) > 0 {
		n, err := target.stream.Write(b)
		if err != nil {
			return err
		}
		if n == 0 {
			return io.ErrShortWrite
		}
		b = b[n:]
	}
	return nil
}

// interruptCommandIO closes owned command/pipe IO on cancellation and returns
// a join function, so no cancellation callback outlives the owning operation.
func interruptCommandIO(ctx context.Context, stream io.Closer) func() {
	done := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		_ = stream.Close() // Cancellation is already the operation's failure.
		close(done)
	})
	return func() {
		if !stop() {
			<-done
		}
	}
}

// Connect waits while a live runner cannot yet accept, but fails when it is
// gone. The caller owns the returned connection and must close it.
func Connect(ctx context.Context, endpoint string) (net.Conn, error) {
	connection, err := connectLocal(ctx, endpoint)
	if err != nil && ctx.Err() != nil {
		return nil, &operationFailure{message: "local connection cancelled", cause: ctx.Err()}
	}
	return connection, err
}
