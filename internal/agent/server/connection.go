package server

import (
	"context"
	"sync"
	"sync/atomic"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/core"
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
type FrameReceiver struct{ outbox *outbox }

// Receive waits for the next frame, or the lagged or closed indication.
// Context cancellation returns ctx.Err without detaching the connection.
func (r *FrameReceiver) Receive(ctx context.Context) (Outgoing, error) {
	for {
		outgoing, changed := r.outbox.next()
		if outgoing != nil {
			return outgoing, nil
		}
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-changed:
		}
	}
}

// TryReceive returns the next outgoing item, or nil if none is waiting.
func (r *FrameReceiver) TryReceive() Outgoing {
	outgoing, _ := r.outbox.next()
	return outgoing
}

// Connection handles one conversation socket's decoded frames. The backend
// supplies its conversation and working directory; clients never send them.
// The socket owner must Detach it when done; the tree's turns keep running.
type Connection[H host.Host] struct {
	server   *Server[H]
	root     core.NodeID
	cwd      string
	resolver ContentResolver
	outbox   *outbox
	detached atomic.Bool
	// Observations are protected by server.mu; edit replies by the tree frame lock.
	observations      map[core.NodeID]*session.Subscription
	stateObservation  *session.Subscription
	editReply         *editReply
	publishedRevision uint64 // Protected by the tree frame lock.
}

// Handle handles a decoded frame to its end. The socket owner calls it in
// arrival order; handling that waits delays the frames behind it. Failures
// are answered as frames, never returned.
func (c *Connection[H]) Handle(ctx context.Context, frame framewire.ClientFrame) {
	c.handle(ctx, frame)
}

// Detach idempotently detaches the connection and releases its outbox when
// the socket is gone. It does not stop the tree's turns.
func (c *Connection[H]) Detach() {
	if c.detached.Swap(true) {
		return
	}
	if tree := c.server.Tree(c.root); tree != nil {
		tree.detach(c)
	}
	c.outbox.close()
}

// outbox bounds a socket's queued frames; a full queue permanently marks lag.
// Its mutex protects only the queue and publication state, never channel IO.
type outbox struct {
	mu       sync.Mutex
	capacity int
	frames   []framewire.ServerFrame
	state    uint8 // 0 open, 1 closed, 2 lagged.
	changed  chan struct{}
}

func (o *outbox) push(frame framewire.ServerFrame) bool {
	o.mu.Lock()
	if o.state != 0 {
		o.mu.Unlock()
		return false
	}
	if len(o.frames) >= o.capacity {
		o.state = 2
		o.frames = nil
	} else {
		o.frames = append(o.frames, frame)
	}
	old := o.changed
	o.changed = make(chan struct{})
	accepted := o.state == 0
	o.mu.Unlock()
	close(old)
	return accepted
}

func (o *outbox) close() {
	o.mu.Lock()
	if o.state != 0 {
		o.mu.Unlock()
		return
	}
	o.state = 1
	old := o.changed
	o.changed = make(chan struct{})
	o.mu.Unlock()
	close(old)
}

func (o *outbox) next() (Outgoing, <-chan struct{}) {
	o.mu.Lock()
	defer o.mu.Unlock()
	if o.state == 2 {
		return &Lagged{}, o.changed
	}
	if len(o.frames) != 0 {
		frame := o.frames[0]
		o.frames[0] = nil
		o.frames = o.frames[1:]
		return &Frame{Frame: frame}, o.changed
	}
	if o.state == 1 {
		return &Closed{}, o.changed
	}
	return nil, o.changed
}
