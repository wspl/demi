package process

//revive:disable:unused-parameter // API checkpoint: stub parameter names document the boundary.

import (
	"context"
	"io"
	"net"

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
func Forward(ctx context.Context, endpoint string, request commandwire.LocalInvocation, stdio Stdio) (commandwire.Completion, error) {
	panic("not written: r-process")
}

// Connect waits while a live runner cannot yet accept, but fails when it is
// gone. The caller owns the returned connection and must close it.
func Connect(ctx context.Context, endpoint string) (net.Conn, error) { panic("not written: r-process") }
