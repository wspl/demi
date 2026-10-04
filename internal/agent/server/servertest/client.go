package servertest

import (
	"context"
	"errors"
	"fmt"
	"testing"
	"time"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/types"
)

// HangGuard bounds a scripted client's wait only to detect hangs.
const HangGuard = 10 * time.Second

// TestClient drives a connection as the backend's socket task would.
// Connect registers connection detachment with test cleanup. The test owns
// the server separately and must shut it down after all clients detach.
type TestClient[H host.Host] struct {
	connection *server.Connection[H]
	frames     *server.Frames
	hangGuard  time.Duration
}

// Connect creates a client of root, whose new tree works in cwd and whose
// frames refer to no files. It registers cleanup with t.
func Connect[H host.Host](t testing.TB, s *server.Server[H], root types.NodeID, cwd string) *TestClient[H] {
	return ConnectWith(t, s, root, cwd, NewFiles())
}

// ConnectWith creates a client whose frames' files resolver resolves.
// It registers cleanup with t.
func ConnectWith[H host.Host](
	t testing.TB,
	s *server.Server[H],
	root types.NodeID,
	cwd string,
	resolver server.ContentResolver,
) *TestClient[H] {
	connection, frames := s.Connect(root, cwd, resolver)
	t.Cleanup(connection.Detach)
	return &TestClient[H]{connection: connection, frames: frames, hangGuard: HangGuard}
}

// SetHangGuard changes the hang deadline for a test using a slower resource.
func (c *TestClient[H]) SetHangGuard(guard time.Duration) { c.hangGuard = guard }

// Send hands frame to the connection and waits until it is handled.
func (c *TestClient[H]) Send(ctx context.Context, frame conversationproto.ClientFrame) {
	c.connection.Handle(ctx, frame)
}

// Next returns the next frame, or nil once the outbox ends or lags.
// Cancellation is returned as an error.
func (c *TestClient[H]) Next(ctx context.Context) (conversationproto.ServerFrame, error) {
	if c.frames.Next(ctx) {
		return c.frames.Frame(), nil
	}
	err := c.frames.Err()
	if errors.Is(err, server.ErrLagged) {
		return nil, nil
	}
	return nil, err
}

// Received returns every frame waiting now.
func (c *TestClient[H]) Received() []conversationproto.ServerFrame { return WaitingFrames(c.frames) }

// Split returns the connection and outbox for observing frames while another
// goroutine handles input. Only one goroutine may read the outbox at a time.
func (c *TestClient[H]) Split() (*server.Connection[H], *server.Frames) {
	return c.connection, c.frames
}

// NextUntil returns frames through the first that until accepts. It returns
// the frames seen and an error if the outbox closes, the context is canceled,
// or the client's hang deadline ends the wait first.
func (c *TestClient[H]) NextUntil(
	ctx context.Context,
	until func(conversationproto.ServerFrame) bool,
) ([]conversationproto.ServerFrame, error) {
	ctx, cancel := context.WithTimeout(ctx, c.hangGuard)
	defer cancel()
	frames := []conversationproto.ServerFrame{}
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

// Connection returns the client's connection handle.
func (c *TestClient[H]) Connection() *server.Connection[H] { return c.connection }

// WaitingFrames returns every frame frames holds now.
func WaitingFrames(frames *server.Frames) []conversationproto.ServerFrame {
	now, cancel := context.WithCancel(context.Background())
	cancel()
	waiting := []conversationproto.ServerFrame{}
	for frames.Next(now) {
		waiting = append(waiting, frames.Frame())
	}
	return waiting
}
