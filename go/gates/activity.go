package gates

import (
	"context"
	"sync"
	"time"

	"golang.org/x/sync/semaphore"
)

// permits is every permit a gate has: a reservation takes them all. It bounds
// the leases held at once far above any real count, on every target.
const permits = 1 << 24

// A Purpose says why a lease holds a gate: what the user asked for, or work the
// system does on its own, such as a checkpoint. Only demand keeps a resource
// from being idle.
type Purpose uint8

// The purposes of a lease.
const (
	Demand Purpose = iota
	Maintenance
)

// A GateState is what holds a gate, as its observers see it.
type GateState struct {
	// Demand is the leases held for demand.
	Demand int
	// Maintenance is the leases held for maintenance.
	Maintenance int
	// Reserved is whether a reservation holds the gate.
	Reserved bool
	// LastDemandEnd is when the last demand lease ended; the zero time until
	// one has.
	LastDemandEnd time.Time
}

// An ActivityHub is a signal that the gates of one user share, changed on every
// change of any of them, so a watch that waits for any of them to change wakes.
// It carries no state of its own: a watch reads [GateState.LastDemandEnd] of
// each gate, which a missed signal cannot lose. The zero ActivityHub is ready
// to use.
type ActivityHub struct {
	changed notifier
}

// Changed returns a channel that is closed at the next change of any gate of
// the hub. Take it before reading the gates, and wait on it after.
func (h *ActivityHub) Changed() <-chan struct{} {
	return h.changed.next()
}

// NewGate returns a gate whose changes also signal the hub.
func (h *ActivityHub) NewGate() *ActivityGate {
	gate := NewActivityGate()
	gate.hub = h
	return gate
}

// An ActivityGate is admission to one resource: many leases at once, or one
// reservation alone. It is a first-in, first-out semaphore with a published
// state: a lease is one permit, a reservation is every permit. A reservation
// that is waiting has taken the permits already free, so every later entrant
// waits behind it and [ActivityGate.TryEnter] is refused; giving up the wait
// returns them.
type ActivityGate struct {
	permits *semaphore.Weighted
	hub     *ActivityHub
	changed notifier
	mu      sync.Mutex
	state   GateState
}

// NewActivityGate returns a gate that nothing holds.
func NewActivityGate() *ActivityGate {
	return &ActivityGate{permits: semaphore.NewWeighted(permits)}
}

// State returns what holds the gate now.
func (g *ActivityGate) State() GateState {
	g.mu.Lock()
	defer g.mu.Unlock()
	return g.state
}

// Changed returns a channel that is closed at the next change of the gate's
// state. Take it before reading the state, and wait on it after.
func (g *ActivityGate) Changed() <-chan struct{} {
	return g.changed.next()
}

// publish changes the state and wakes those who watch it, and the hub.
func (g *ActivityGate) publish(change func(*GateState)) {
	g.mu.Lock()
	change(&g.state)
	g.mu.Unlock()
	g.changed.notify()
	if g.hub != nil {
		g.hub.changed.notify()
	}
}

// Enter holds a lease once no reservation holds or waits for the gate ahead of
// this call. It returns the context's error, and holds nothing, when ctx ends
// first.
func (g *ActivityGate) Enter(ctx context.Context, purpose Purpose) (*Lease, error) {
	if err := g.permits.Acquire(ctx, 1); err != nil {
		return nil, err
	}
	return g.admit(purpose), nil
}

// TryEnter holds a lease now, or none while a reservation holds or waits.
func (g *ActivityGate) TryEnter(purpose Purpose) (*Lease, bool) {
	if !g.permits.TryAcquire(1) {
		return nil, false
	}
	return g.admit(purpose), true
}

func (g *ActivityGate) admit(purpose Purpose) *Lease {
	g.publish(func(state *GateState) {
		switch purpose {
		case Demand:
			state.Demand++
		case Maintenance:
			state.Maintenance++
		}
	})
	return &Lease{gate: g, purpose: purpose}
}

// Reserve holds the gate alone once every lease ahead of it has ended. Leases
// that arrive while it waits wait behind it. It returns the context's error,
// and holds nothing, when ctx ends first.
func (g *ActivityGate) Reserve(ctx context.Context) (*Reservation, error) {
	if err := g.permits.Acquire(ctx, permits); err != nil {
		return nil, err
	}
	return g.hold(), nil
}

// TryReserve holds the gate alone now, or none while anything holds or waits
// for it.
func (g *ActivityGate) TryReserve() (*Reservation, bool) {
	if !g.permits.TryAcquire(permits) {
		return nil, false
	}
	return g.hold(), true
}

func (g *ActivityGate) hold() *Reservation {
	g.publish(func(state *GateState) { state.Reserved = true })
	return &Reservation{gate: g}
}

// A Lease is one piece of work in progress; releasing it ends the work's hold.
type Lease struct {
	gate    *ActivityGate
	purpose Purpose
	once    sync.Once
}

// Purpose returns why the lease holds the gate.
func (l *Lease) Purpose() Purpose {
	return l.purpose
}

// Release ends the lease. The state no longer counts the lease before the gate
// admits anyone else, so a reservation it lets in never sees it.
func (l *Lease) Release() {
	l.once.Do(func() {
		l.gate.publish(func(state *GateState) {
			switch l.purpose {
			case Demand:
				state.Demand--
				if state.Demand == 0 {
					state.LastDemandEnd = time.Now()
				}
			case Maintenance:
				state.Maintenance--
			}
		})
		l.gate.permits.Release(1)
	})
}

// A Reservation is the gate held alone, for a transition that needs it quiet;
// releasing it admits the entrants waiting behind it.
type Reservation struct {
	gate *ActivityGate
	once sync.Once
}

// Release ends the reservation. The state shows the gate free before the gate
// admits anyone.
func (r *Reservation) Release() {
	// The state is published before the permits are freed: an entrant that the
	// release lets in must never read the gate as reserved.
	r.once.Do(func() {
		r.gate.publish(func(state *GateState) { state.Reserved = false })
		r.gate.permits.Release(permits)
	})
}
