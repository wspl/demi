package server

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/host"
)

// Outgoing is what the outbox has next for the socket.
//
//sumtype:decl
type Outgoing interface{ outgoing() }

// Frame carries one server frame.
type Frame struct{ Frame framewire.ServerFrame }

// Lagged means the client fell behind by a full outbox; close its socket.
type Lagged struct{}

// Closed means the connection is gone.
type Closed struct{}

func (*Frame) outgoing()  {}
func (*Lagged) outgoing() {}
func (*Closed) outgoing() {}

// FrameReceiver is the receiving end of a connection's bounded outbox.
// One socket reader owns it. Detaching the connection releases the outbox.
type FrameReceiver struct{}

// Receive waits for the next frame, or the lagged or closed indication.
// Context cancellation returns ctx.Err without detaching the connection.
func (r *FrameReceiver) Receive(ctx context.Context) (Outgoing, error) {
	panic("not written: a-server")
}

// TryReceive returns the next outgoing item, or nil if none is waiting.
func (r *FrameReceiver) TryReceive() Outgoing { panic("not written: a-server") }

// Connection handles one conversation socket's decoded frames. The backend
// supplies its conversation and working directory; clients never send them.
// The socket owner must Detach it when done; the tree's turns keep running.
type Connection[H host.Host] struct{}

// Handle handles a decoded frame to its end. The socket owner calls it in
// arrival order; handling that waits delays the frames behind it. Failures
// are answered as frames, never returned.
func (c *Connection[H]) Handle(ctx context.Context, frame framewire.ClientFrame) {
	panic("not written: a-server")
}

// Detach idempotently detaches the connection and releases its outbox when
// the socket is gone. It does not stop the tree's turns.
func (c *Connection[H]) Detach() { panic("not written: a-server") }
