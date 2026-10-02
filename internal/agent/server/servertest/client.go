package servertest

import (
	"context"
	"fmt"
	"testing"
	"time"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/host"
)

// HangGuard bounds a scripted client's wait only to detect hangs.
const HangGuard = 10 * time.Second

// TestClient drives a connection as the backend's socket task would.
// Connect registers connection detachment with test cleanup. The test owns
// the server separately and must shut it down after all clients detach.
type TestClient[H host.Host] struct {
	connection *server.Connection[H]
	frames     *server.FrameReceiver
	hangGuard  time.Duration
}

// Connect creates a client of root, whose new tree works in cwd and whose
// frames refer to no files. It registers cleanup with t.
func Connect[H host.Host](t testing.TB, s *server.Server[H], root core.NodeID, cwd string) *TestClient[H] {
	return ConnectWith(t, s, root, cwd, NewFiles())
}

// ConnectWith creates a client whose frames' files resolver resolves.
// It registers cleanup with t.
func ConnectWith[H host.Host](t testing.TB, s *server.Server[H], root core.NodeID, cwd string, resolver server.ContentResolver) *TestClient[H] {
	connection, frames := s.Connect(root, cwd, resolver)
	t.Cleanup(connection.Detach)
	return &TestClient[H]{connection: connection, frames: frames, hangGuard: HangGuard}
}

// SetHangGuard changes the hang deadline for a test using a slower resource.
func (c *TestClient[H]) SetHangGuard(guard time.Duration) { c.hangGuard = guard }

// Send hands frame to the connection and waits until it is handled.
func (c *TestClient[H]) Send(ctx context.Context, frame framewire.ClientFrame) {
	c.connection.Handle(ctx, frame)
}

// Next returns the next frame, or nil once the outbox closes or lags.
// Cancellation is returned as an error.
func (c *TestClient[H]) Next(ctx context.Context) (framewire.ServerFrame, error) {
	out, err := c.frames.Receive(ctx)
	if err != nil {
		return nil, err
	}
	switch v := out.(type) {
	case *server.Frame:
		return v.Frame, nil
	case *server.Lagged, *server.Closed:
		return nil, nil
	}
	return nil, nil
}

// Received returns every frame waiting now.
func (c *TestClient[H]) Received() []framewire.ServerFrame { return WaitingFrames(c.frames) }

// Split returns the connection and outbox for observing frames while another
// goroutine handles input. Only one goroutine may read the outbox at a time.
func (c *TestClient[H]) Split() (*server.Connection[H], *server.FrameReceiver) {
	return c.connection, c.frames
}

// NextUntil returns frames through the first that until accepts. It returns
// the frames seen and an error if the outbox closes, the context is canceled,
// or the client's hang deadline ends the wait first.
func (c *TestClient[H]) NextUntil(ctx context.Context, until func(framewire.ServerFrame) bool) ([]framewire.ServerFrame, error) {
	ctx, cancel := context.WithTimeout(ctx, c.hangGuard)
	defer cancel()
	frames := []framewire.ServerFrame{}
	for {
		frame, err := c.Next(ctx)
		if err != nil {
			return frames, fmt.Errorf("waiting for a frame after %v: %w", frames, err)
		}
		if frame == nil {
			return frames, fmt.Errorf("the connection closed before a frame ended the wait: %v", frames)
		}
		frames = append(frames, frame)
		if until(frame) {
			return frames, nil
		}
	}
}

// Outgoing returns the next outbox item, including lagged or closed.
func (c *TestClient[H]) Outgoing(ctx context.Context) (server.Outgoing, error) {
	return c.frames.Receive(ctx)
}

// Connection returns the client's connection handle.
func (c *TestClient[H]) Connection() *server.Connection[H] { return c.connection }

// WaitingFrames returns every frame outbox holds now.
func WaitingFrames(outbox *server.FrameReceiver) []framewire.ServerFrame {
	frames := []framewire.ServerFrame{}
	for {
		outgoing := outbox.TryReceive()
		if outgoing == nil {
			return frames
		}
		switch v := outgoing.(type) {
		case *server.Frame:
			frames = append(frames, v.Frame)
		case *server.Lagged, *server.Closed:
			return frames
		}
	}
}
