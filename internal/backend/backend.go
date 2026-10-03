package backend

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"
	"net/netip"

	"github.com/wspl/demi/internal/backend/usershard"
)

// Backend is a running backend. Its owner must call Close and wait for all
// services, shards, connections and storage to end, including after ctx ends.
type Backend struct{}

// Start starts the backend. It serves once this returns: the data directory
// and the instance secret, then the databases and the object store, the
// shared services and the shards, and the listener last. A failed start
// releases everything acquired by that attempt. ctx owns the backend lifetime.
func Start(ctx context.Context, config Config) (*Backend, error) {
	panic("not written: b-backend")
}

// LocalAddr is the address the listener is bound to.
func (b *Backend) LocalAddr() netip.AddrPort { panic("not written: b-backend") }

// Close shuts the backend down. The listener closes first, so no new work
// starts and a new request on an open connection answers 503 backend_closing;
// every step runs even when an earlier one fails. Failures are returned as
// ShutdownErrors. Cleanup uses a context that outlives the canceled lifetime.
func (b *Backend) Close(ctx context.Context) error { panic("not written: b-backend") }

// Services borrows the shared service handles for backendtest's commit holds.
// Callers must not replace handles or close services independently of Backend.
func (b *Backend) Services() *usershard.Services { panic("not written: b-backend") }

// Shards borrows user routing for backendtest's file gates, exposes and
// retention passes. Callers must not close it independently of Backend.
func (b *Backend) Shards() *usershard.Shards { panic("not written: b-backend") }
