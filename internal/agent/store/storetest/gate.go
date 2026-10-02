package storetest

import (
	"context"
	"sync"
)

// StoreGate holds store calls as a slow database would. The acquiring test
// must release it during cleanup; canceled calls leave the gate promptly.
type StoreGate struct {
	// mu protects the count and publication. Notifications are closed outside it.
	mu      sync.Mutex
	waiting int
	open    bool
	changed chan struct{}
}

// Waiting reports how many calls are waiting.
func (g *StoreGate) Waiting() int {
	g.mu.Lock()
	defer g.mu.Unlock()
	return g.waiting
}

// Wait waits for at least count calls to be held, without polling or sleeping.
func (g *StoreGate) Wait(ctx context.Context, count int) error {
	for {
		g.mu.Lock()
		if g.waiting >= count {
			g.mu.Unlock()
			return nil
		}
		if g.changed == nil {
			g.changed = make(chan struct{})
		}
		changed := g.changed
		g.mu.Unlock()
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-changed:
		}
	}
}

// Release lets current and future calls through; it is idempotent.
func (g *StoreGate) Release() {
	g.mu.Lock()
	g.open = true
	old := g.changed
	g.changed = make(chan struct{})
	g.mu.Unlock()
	if old != nil {
		close(old)
	}
}

// pass holds one store call until release or cancellation, tracking its lifetime.
func (g *StoreGate) pass(ctx context.Context) error {
	if g == nil {
		return ctx.Err()
	}
	g.mu.Lock()
	g.waiting++
	old := g.changed
	g.changed = make(chan struct{})
	g.mu.Unlock()
	if old != nil {
		close(old)
	}
	defer func() {
		g.mu.Lock()
		g.waiting--
		old := g.changed
		g.changed = make(chan struct{})
		g.mu.Unlock()
		close(old)
	}()
	for {
		g.mu.Lock()
		open, changed := g.open, g.changed
		g.mu.Unlock()
		if open {
			return ctx.Err()
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-changed:
		}
	}
}
