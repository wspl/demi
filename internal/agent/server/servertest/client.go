package servertest

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"
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
type TestClient[H host.Host] struct{}

// Connect creates a client of root, whose new tree works in cwd and whose
// frames refer to no files. It registers cleanup with t.
func Connect[H host.Host](t testing.TB, s *server.Server[H], root core.NodeID, cwd string) *TestClient[H] {
	panic("not written: a-server")
}

// ConnectWith creates a client whose frames' files resolver resolves.
// It registers cleanup with t.
func ConnectWith[H host.Host](t testing.TB, s *server.Server[H], root core.NodeID, cwd string, resolver server.ContentResolver) *TestClient[H] {
	panic("not written: a-server")
}

// SetHangGuard changes the hang deadline for a test using a slower resource.
func (c *TestClient[H]) SetHangGuard(guard time.Duration) { panic("not written: a-server") }

// Send hands frame to the connection and waits until it is handled.
func (c *TestClient[H]) Send(ctx context.Context, frame framewire.ClientFrame) {
	panic("not written: a-server")
}

// Next returns the next frame, or nil once the outbox closes or lags.
// Cancellation is returned as an error.
func (c *TestClient[H]) Next(ctx context.Context) (framewire.ServerFrame, error) {
	panic("not written: a-server")
}

// Received returns every frame waiting now.
func (c *TestClient[H]) Received() []framewire.ServerFrame { panic("not written: a-server") }

// Split returns the connection and outbox for observing frames while another
// goroutine handles input. Only one goroutine may read the outbox at a time.
func (c *TestClient[H]) Split() (*server.Connection[H], *server.FrameReceiver) {
	panic("not written: a-server")
}

// NextUntil returns frames through the first that until accepts. It returns
// the frames seen and an error if the outbox closes, the context is canceled,
// or the client's hang deadline ends the wait first.
func (c *TestClient[H]) NextUntil(ctx context.Context, until func(framewire.ServerFrame) bool) ([]framewire.ServerFrame, error) {
	panic("not written: a-server")
}

// Outgoing returns the next outbox item, including lagged or closed.
func (c *TestClient[H]) Outgoing(ctx context.Context) (server.Outgoing, error) {
	panic("not written: a-server")
}

// Connection returns the client's connection handle.
func (c *TestClient[H]) Connection() *server.Connection[H] { panic("not written: a-server") }

// WaitingFrames returns every frame outbox holds now.
func WaitingFrames(outbox *server.FrameReceiver) []framewire.ServerFrame {
	panic("not written: a-server")
}
