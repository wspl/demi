package cmdpkgs

import (
	"context"
	"errors"
	"io"
	"sync"
)

// Decision is a registry retention decision, exposed for deterministic tests.
type Decision uint8

const (
	// Leased means a lease still holds the service; no status was asked.
	Leased Decision = iota
	// Asks means a status request precedes a later decision.
	Asks
	// HoldsConversations means the service holds state and stays resident.
	HoldsConversations
	// Unanswered means status failed and the service stays resident.
	Unanswered
	// Stops means the service holds nothing and stops.
	Stops
)

// DecisionEvent pairs a decision with the executable's digest.
type DecisionEvent struct {
	// Digest identifies the artifact whose decision changed.
	Digest string
	// Decision is the new authorization decision.
	Decision Decision
}

// Decisions observes future decisions in order until ctx or the registry ends.
// This test observation stream has the Rust broadcast's 64-event capacity.
func (r *ServiceRegistry) Decisions(ctx context.Context) *DecisionReceiver {
	d := &r.decisions
	d.mu.Lock()
	defer d.mu.Unlock()
	return &DecisionReceiver{log: d, ctx: ctx, next: d.next}
}

// DecisionReceiver observes registry decisions without holding services alive.
type DecisionReceiver struct {
	log  *decisionLog
	ctx  context.Context
	next uint64
}
type decisionLog struct {
	mu      sync.Mutex
	events  [64]DecisionEvent
	next    uint64
	closed  bool
	changed chan struct{}
}

// notifyLocked requires the decision mutex so ring publication and notification stay atomic.
func (d *decisionLog) notifyLocked() {
	if d.changed != nil {
		close(d.changed)
	}
	d.changed = make(chan struct{})
}

func (d *decisionLog) add(digest string, decision Decision) {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.events[d.next%64] = DecisionEvent{Digest: digest, Decision: decision}
	d.next++
	d.notifyLocked()
}

func (d *decisionLog) close() {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.closed = true
	d.notifyLocked()
}

// Next waits for a decision, reporting lag or closure as an error.
func (r *DecisionReceiver) Next(ctx context.Context) (DecisionEvent, error) {
	d := r.log
	for {
		d.mu.Lock()
		if d.next-r.next > 64 {
			r.next = d.next - 64
			d.mu.Unlock()
			return DecisionEvent{}, errors.New("registry decision receiver lagged")
		}
		if r.next < d.next {
			event := d.events[r.next%64]
			r.next++
			d.mu.Unlock()
			return event, nil
		}
		if d.closed {
			d.mu.Unlock()
			return DecisionEvent{}, io.EOF
		}
		if d.changed == nil {
			d.changed = make(chan struct{})
		}
		changed := d.changed
		d.mu.Unlock()
		select {
		case <-ctx.Done():
			return DecisionEvent{}, ctx.Err()
		case <-r.ctx.Done():
			return DecisionEvent{}, r.ctx.Err()
		case <-changed:
		}
	}
}
