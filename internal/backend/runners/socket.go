package runners

import (
	"context"
	"errors"
	"io"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/runnerproto"
)

// Socket owns the runner connection's single reader, from hello through serving.
// Its owner must Release it, including when adoption fails. Reads may be handed
// between owners, but only one consumer may read at a time.
type Socket struct {
	conn   *websocket.Conn
	cancel context.CancelFunc
	frames chan runnerFrame
	done   chan struct{}
}

type runnerFrame struct {
	kind websocket.MessageType
	data []byte
	err  error
}

// NewSocket takes ownership of conn and starts its lifetime reader.
func NewSocket(ctx context.Context, conn *websocket.Conn) *Socket {
	ctx, cancel := context.WithCancel(ctx)
	s := &Socket{conn: conn, cancel: cancel, frames: make(chan runnerFrame), done: make(chan struct{})}
	conn.SetReadLimit(runnerproto.MaxMessageBytes)
	go func() {
		defer close(s.done)
		defer close(s.frames)
		for {
			kind, data, err := conn.Read(ctx)
			select {
			case s.frames <- runnerFrame{kind: kind, data: data, err: err}:
			case <-ctx.Done():
				return
			}
			if err != nil {
				return
			}
		}
	}()
	return s
}

// Read receives a frame without tying the lifetime reader to this wait's context.
// Canceling a hello/claim wait therefore leaves the same reader for adoption.
func (s *Socket) Read(ctx context.Context) (websocket.MessageType, []byte, error) {
	select {
	case frame, ok := <-s.frames:
		if !ok {
			return 0, nil, io.EOF
		}
		return frame.kind, frame.data, frame.err
	case <-ctx.Done():
		return 0, nil, ctx.Err()
	}
}

// Receive supplies binary runner protocol frames to the link driver.
func (s *Socket) Receive(ctx context.Context) ([]byte, error) {
	kind, data, err := s.Read(ctx)
	if err == nil && kind != websocket.MessageBinary {
		err = errors.New("the runner sent a text frame")
	}
	return data, err
}

// Send writes a binary runner protocol frame.
func (s *Socket) Send(ctx context.Context, frame []byte) error {
	return s.conn.Write(ctx, websocket.MessageBinary, frame)
}

// Close completes the connection-local close handshake. The owner still releases
// the Socket to join its reader, which may be waiting to deliver a frame.
func (s *Socket) Close(code websocket.StatusCode, reason string) error {
	return s.conn.Close(code, reason)
}

// Release closes the transport and joins the reader. It is idempotent.
func (s *Socket) Release() {
	s.cancel()
	// No live work remains and there is no recipient for a transport close error.
	_ = s.conn.CloseNow()
	<-s.done
}
