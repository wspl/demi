package server

import (
	"context"
	"errors"
	"io"
	"sync"
	"sync/atomic"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/types"
)

// ErrLagged means the client fell behind by a full outbox; close its socket.
var ErrLagged = errors.New("the client fell behind by a full outbox")

// Frames reads a connection's bounded outbox. One socket reader owns it.
// Detaching the connection ends it once its queued frames are read.
type Frames struct {
	outbox *outbox
	frame  conversationproto.ServerFrame
	err    error
}

// Next waits for the next frame and reports whether there is one. A frame
// already waiting is returned even when ctx is done, so a done context reads
// without waiting. Next returns false at the end, after a lag, or when ctx
// ends first; Err tells which. After a cancellation Next may be called again.
func (f *Frames) Next(ctx context.Context) bool {
	f.frame = nil
	f.err = nil
	for {
		frame, changed, err := f.outbox.next()
		if frame != nil {
			f.frame = frame
			return true
		}
		if errors.Is(err, io.EOF) {
			return false
		}
		if err != nil {
			f.err = err
			return false
		}
		select {
		case <-ctx.Done():
			f.err = ctx.Err()
			return false
		case <-changed:
		}
	}
}

// Frame returns the frame the last Next found.
func (f *Frames) Frame() conversationproto.ServerFrame {
	return f.frame
}

// Err returns ErrLagged after a lag, the context's error after a cancelled
// wait, and nil at the end of the outbox.
func (f *Frames) Err() error {
	return f.err
}

// Connection handles one conversation socket's decoded frames. The backend
// supplies its conversation and working directory; clients never send them.
// The socket owner must Detach it when done; the tree's turns keep running.
type Connection[H host.Host] struct {
	server   *Server[H]
	root     types.NodeID
	cwd      string
	resolver ContentResolver
	outbox   *outbox
	detached atomic.Bool
	// Observations are protected by server.mu; edit replies by the tree frame lock.
	observations      map[types.NodeID]*session.Subscription
	stateObservation  *session.Subscription
	editReply         *editReply
	publishedRevision uint64 // Protected by the tree frame lock.
}

// Handle handles a decoded frame to its end. The socket owner calls it in
// arrival order; handling that waits delays the frames behind it. Failures
// are answered as frames, never returned.
func (c *Connection[H]) Handle(ctx context.Context, frame conversationproto.ClientFrame) {
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
	frames   []conversationproto.ServerFrame
	state    uint8 // 0 open, 1 closed, 2 lagged.
	changed  chan struct{}
}

func (o *outbox) push(frame conversationproto.ServerFrame) bool {
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

// next returns the next queued frame. With none queued it returns ErrLagged
// after a lag, io.EOF once the outbox is closed, and otherwise the channel
// that closes on the next change.
func (o *outbox) next() (conversationproto.ServerFrame, <-chan struct{}, error) {
	o.mu.Lock()
	defer o.mu.Unlock()
	if o.state == 2 {
		return nil, nil, ErrLagged
	}
	if len(o.frames) != 0 {
		frame := o.frames[0]
		o.frames[0] = nil
		o.frames = o.frames[1:]
		return frame, nil, nil
	}
	if o.state == 1 {
		return nil, nil, io.EOF
	}
	return nil, o.changed, nil
}
