package runnerstest

import (
	"context"
	"testing"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
)

// Connection owns a bound connection's driver over a test's transport.
// Serve registers cancellation and a join with the test's cleanup.
type Connection struct {
	cancel context.CancelFunc
	done   chan struct{}
	end    remotehost.LinkEnd
}

// Serve starts serving without a socket. It owns the driver in serving; incoming
// and outgoing must unblock on cancellation and their resources stay with the
// caller. Cleanup cancels and joins the driver, including on a failed test.
func Serve(t testing.TB, serving *runners.Serving, incoming remotehost.FrameSource, outgoing remotehost.FrameSink) *Connection {
	t.Helper()
	ctx, cancel := context.WithCancel(t.Context())
	c := &Connection{cancel: cancel, done: make(chan struct{})}
	t.Cleanup(func() {
		if err := c.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	go func() {
		defer close(c.done)
		c.end = serving.TestingServe(ctx, incoming, outgoing)
	}()
	return c
}

// End waits for the connection's end, or for ctx to be cancelled.
func (c *Connection) End(ctx context.Context) (remotehost.LinkEnd, error) {
	select {
	case <-c.done:
		return c.end, nil
	case <-ctx.Done():
		return remotehost.LinkEnd{}, ctx.Err()
	}
}

// Close cancels and joins the connection, including its slot and last-seen cleanup.
// It may be called before the test's registered cleanup and is idempotent.
func (c *Connection) Close(ctx context.Context) error {
	c.cancel()
	_, err := c.End(ctx)
	return err
}
