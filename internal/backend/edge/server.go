package edge

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the public boundary is merged.

import (
	"context"
	"net/netip"
)

// Edge is the listener and the server over it. Obtain it with Start and share
// its pointer; its owner must call Close before disposing shared services.
type Edge struct{}

// Start binds address and starts serving the API and, when webDirectory is
// nonempty, the built web app in that directory. Port zero selects a free port.
// It publishes the listening address to state.Services.PublicURL before serving
// the first request. A failed start releases resources acquired by that attempt.
// ctx owns the edge's lifetime; cancellation closes its connections and listener.
// The caller must still call Close to join its work.
func Start(ctx context.Context, address netip.AddrPort, state AppState, webDirectory string) (*Edge, error) {
	panic("not written: b-edge")
}

// LocalAddr returns the bound listener address, including the selected port.
func (e *Edge) LocalAddr() netip.AddrPort {
	panic("not written: b-edge")
}

// StopAccepting closes the listener. Open connections go on serving, but a new
// request on one answers 503 backend_closing. Runner pipes remain available
// because backend shutdown needs them. This method is idempotent.
func (e *Edge) StopAccepting() {
	panic("not written: b-edge")
}

// Close closes the listener and connections still open, such as a download's,
// and waits for the server and its request and copy workers to end. It is
// idempotent. If ctx ends before the join finishes, shutdown continues; the owner
// must call Close again with a live context to finish joining before disposing
// shared services. Shard-owned sockets are drained by the shards before Close.
func (e *Edge) Close(ctx context.Context) error {
	panic("not written: b-edge")
}
