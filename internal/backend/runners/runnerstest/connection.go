package runnerstest

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"
	"testing"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
)

// Connection owns a bound connection's driver over a test's transport.
// Serve registers cancellation and a join with the test's cleanup.
type Connection struct{}

// Serve starts serving without a socket. It owns the driver in serving; incoming
// and outgoing must unblock on cancellation and their resources stay with the
// caller. Cleanup cancels and joins the driver, including on a failed test.
func Serve(t testing.TB, serving *runners.Serving, incoming remotehost.FrameSource, outgoing remotehost.FrameSink) *Connection {
	panic("not written: b-runners")
}

// End waits for the connection's end, or for ctx to be cancelled.
func (c *Connection) End(ctx context.Context) (remotehost.LinkEnd, error) {
	panic("not written: b-runners")
}

// Close cancels and joins the connection, including its slot and last-seen cleanup.
// It may be called before the test's registered cleanup and is idempotent.
func (c *Connection) Close(ctx context.Context) error { panic("not written: b-runners") }
