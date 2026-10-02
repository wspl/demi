package cloud

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"

	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/webapi"
)

// Client owns the machine manager's socket, opened on first use and again by
// the next call after a drop. Every in-flight call fails on a drop; requests
// are never replayed. Concurrent calls match replies by request ID. Share its
// pointer; the owner must Close it to join its supervisor and reader.
type Client struct{}

// NewClient creates a client of socket and the sole receiver of death events:
// the devices whose sandboxes exited without being asked to stop. ctx belongs
// to the backend lifetime. The backend routes events until the client closes
// the channel; the queue holds 256 events and backpressures the reader.
func NewClient(ctx context.Context, socket string) (*Client, <-chan webapi.DeviceID) {
	panic("not written: b-cloud")
}

// Call runs params on the manager and decodes the reply through its operation
// contract. Cancellation ends this wait without retrying or undoing the
// manager's operation. Lifecycle callers query state before retrying a lost call.
func Call[T any](ctx context.Context, client *Client, params machinewire.Operation[T]) (T, error) {
	panic("not written: b-cloud")
}

// Disconnect reconciles over the live connection, if any, then disconnects
// even if reconciliation failed. It returns that failure; the manager keeps
// running and a later call connects again. No connection is opened solely to
// disconnect. This is the reusable close operation of the Rust client.
func (c *Client) Disconnect(ctx context.Context) error { panic("not written: b-cloud") }

// Close permanently stops new calls, reconciles and disconnects a live socket,
// and joins every owned goroutine. It is idempotent. The owner calls it with a
// usable cleanup context after all Clouds have closed. It replaces Rust's
// final client drop as well as its shutdown disconnect.
func (c *Client) Close(ctx context.Context) error { panic("not written: b-cloud") }
