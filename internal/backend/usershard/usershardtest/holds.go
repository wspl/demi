package usershardtest

import (
	"context"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/backend/usershard"
)

// StepHolds holds a flow at at most one of its named steps. Its zero value is ready.
type StepHolds[S comparable] struct {
	mu   sync.Mutex
	step S
	held *StepHold
}

// Hold holds step until Release or test cleanup. Replacing a hold leaves its
// owner responsible for releasing it; cleanup also releases failed tests' holds.
func (h *StepHolds[S]) Hold(t testing.TB, step S) *StepHold {
	t.Helper()
	hold := &StepHold{released: make(chan struct{}), changed: make(chan struct{})}
	t.Cleanup(hold.Release)
	h.mu.Lock()
	h.step = step
	h.held = hold
	h.mu.Unlock()
	return hold
}

// Pass waits while step is held, counting its arrival even if canceled later.
func (h *StepHolds[S]) Pass(ctx context.Context, step S) error {
	h.mu.Lock()
	hold := h.held
	if h.step != step {
		hold = nil
	}
	h.mu.Unlock()
	if hold == nil {
		return nil
	}
	hold.mu.Lock()
	hold.arrived++
	changed := hold.changed
	hold.changed = make(chan struct{})
	hold.mu.Unlock()
	close(changed)
	select {
	case <-hold.released:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// StepHold is a test's hold at one flow step; its test cleanup releases it.
type StepHold struct {
	mu       sync.Mutex
	arrived  int
	changed  chan struct{}
	released chan struct{}
	once     sync.Once
}

// UntilArrived waits for count arrivals, including callers that left meanwhile.
func (h *StepHold) UntilArrived(ctx context.Context, count int) error {
	for {
		h.mu.Lock()
		arrived, changed := h.arrived, h.changed
		h.mu.Unlock()
		if arrived >= count {
			return nil
		}
		select {
		case <-changed:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
}

// Release lets waiting and future passes through. It is idempotent.
func (h *StepHold) Release() { h.once.Do(func() { close(h.released) }) }

// Hooks connects independently held runner and page flows to shared services.
// Set Services.Hooks before starting routing or accepting any sockets.
type Hooks struct {
	Hellos StepHolds[usershard.HelloStep]
	Syncs  StepHolds[usershard.SyncStep]
}

// Hello passes the runner flow's step.
func (h *Hooks) Hello(ctx context.Context, step usershard.HelloStep) error {
	return h.Hellos.Pass(ctx, step)
}

// Sync passes the page flow's step.
func (h *Hooks) Sync(ctx context.Context, step usershard.SyncStep) error {
	return h.Syncs.Pass(ctx, step)
}

var _ usershard.FlowHooks = (*Hooks)(nil)
