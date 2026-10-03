package gates

import (
	"context"
	"fmt"
	"sync"
	"sync/atomic"
	"time"

	"golang.org/x/sync/semaphore"
)

const activityPermits int64 = 1 << 24

// Purpose distinguishes user demand from background maintenance.
type Purpose uint8

const (
	// Demand keeps the resource active.
	Demand Purpose = iota
	// Maintenance does not reset the resource's idle time.
	Maintenance
)

// State is an immutable publication. A zero LastDemandEnd means demand has
// never ended. Changed closes after a newer state has been published.
type State struct {
	// Demand counts admitted leases that keep the resource active.
	Demand uint32
	// Maintenance counts admitted leases that do not reset idle time.
	Maintenance uint32
	// Reserved reports an admitted exclusive reservation.
	Reserved bool
	// LastDemandEnd records when the final demand lease ended; zero means never.
	LastDemandEnd time.Time
	changed       chan struct{}
}

// Changed returns the notification channel belonging to this snapshot.
func (s State) Changed() <-chan struct{} { return s.changed }

// Hub notifies observers when any attached Activity changes. Its zero value
// is ready for use. A Hub must not be copied after first use.
type Hub struct {
	mu      sync.Mutex // Protects the notification channel.
	changed chan struct{}
}

// Subscribe returns a channel closed by the next change. Subscribe before
// reading the activities, then reload them after notification.
func (h *Hub) Subscribe() <-chan struct{} {
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.changed == nil {
		h.changed = make(chan struct{})
	}
	return h.changed
}

// bump announces an activity publication to the hub's observers.
func (h *Hub) bump() {
	h.mu.Lock()
	old := h.changed
	h.changed = make(chan struct{})
	h.mu.Unlock()
	if old != nil {
		close(old)
	}
}

// Activity admits concurrent leases or an exclusive reservation. Construct it
// with NewActivity; do not copy it. It starts no goroutines.
type Activity struct {
	permits        *semaphore.Weighted
	hub            *Hub
	mu             sync.Mutex // Serializes state publications and waiting observations.
	state          atomic.Pointer[State]
	waiting        int
	waitingChanged chan struct{}
}

// NewActivity constructs an activity, optionally attached to hub (nil means
// no aggregate notifications).
func NewActivity(hub *Hub) *Activity {
	a := &Activity{
		permits:        semaphore.NewWeighted(activityPermits),
		hub:            hub,
		waitingChanged: make(chan struct{}),
	}
	a.state.Store(&State{changed: make(chan struct{})})
	return a
}

// State returns a value and its notification channel from one publication.
func (a *Activity) State() State { return *a.state.Load() }

// publish updates activity state before announcing the replacement.
func (a *Activity) publish(change func(*State)) {
	a.mu.Lock()
	next := *a.state.Load()
	change(&next)
	old := next.changed
	next.changed = make(chan struct{})
	a.state.Store(&next)
	a.mu.Unlock()
	close(old)
	if a.hub != nil {
		a.hub.bump()
	}
}

// countWaiting tracks activity entrants until admission or cancellation.
func (a *Activity) countWaiting(delta int) {
	a.mu.Lock()
	a.waiting += delta
	old := a.waitingChanged
	a.waitingChanged = make(chan struct{})
	a.mu.Unlock()
	close(old)
}

// TestingWaiting is the observation bridge for gatestest.Waiting. Product
// callers should use State; waiting entrants are not admitted activity.
func (a *Activity) TestingWaiting() (int, <-chan struct{}) {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.waiting, a.waitingChanged
}

// Enter waits for a lease in FIFO order. Cancellation relinquishes its place.
func (a *Activity) Enter(ctx context.Context, purpose Purpose) (*Lease, error) {
	if err := ctx.Err(); err != nil {
		return nil, fmt.Errorf("enter activity: %w", err)
	}
	if !a.permits.TryAcquire(1) {
		a.countWaiting(1)
		err := a.permits.Acquire(ctx, 1)
		a.countWaiting(-1)
		if err != nil {
			return nil, fmt.Errorf("enter activity: %w", err)
		}
	}
	return a.admit(purpose), nil
}

// TryEnter admits immediately, or returns nil behind a reservation or waiter.
func (a *Activity) TryEnter(purpose Purpose) *Lease {
	if !a.permits.TryAcquire(1) {
		return nil
	}
	return a.admit(purpose)
}

// admit records an activity lease after its permit has been acquired.
func (a *Activity) admit(purpose Purpose) *Lease {
	a.publish(func(s *State) {
		if purpose == Demand {
			s.Demand++
		} else {
			s.Maintenance++
		}
	})
	return &Lease{gate: a, purpose: purpose}
}

// Reserve takes every permit, holding back later entrants while it drains.
func (a *Activity) Reserve(ctx context.Context) (*Reservation, error) {
	if err := a.permits.Acquire(ctx, activityPermits); err != nil {
		return nil, fmt.Errorf("reserve activity: %w", err)
	}
	return a.hold(), nil
}

// TryReserve succeeds only when there are no holders or queued predecessors.
func (a *Activity) TryReserve() *Reservation {
	if !a.permits.TryAcquire(activityPermits) {
		return nil
	}
	return a.hold()
}

// hold publishes an exclusive activity reservation.
func (a *Activity) hold() *Reservation {
	a.publish(func(s *State) { s.Reserved = true })
	return &Reservation{gate: a}
}

// Lease owns one activity permit. Pass its pointer to transfer ownership;
// do not copy the value. Release must be called even after cancellation.
type Lease struct {
	once    sync.Once
	gate    *Activity
	purpose Purpose
}

// Purpose reports why this lease holds the activity.
func (l *Lease) Purpose() Purpose { return l.purpose }

// Release ends the lease exactly once, from any goroutine.
func (l *Lease) Release() {
	l.once.Do(func() {
		l.gate.publish(func(s *State) {
			if l.purpose == Demand {
				s.Demand--
				if s.Demand == 0 {
					s.LastDemandEnd = time.Now()
				}
			} else {
				s.Maintenance--
			}
		})
		l.gate.permits.Release(1)
	})
}

// Reservation owns exclusive activity admission. It must not be copied.
type Reservation struct {
	once sync.Once
	gate *Activity
}

// Release ends the reservation exactly once, from any goroutine.
func (r *Reservation) Release() {
	r.once.Do(func() {
		r.gate.publish(func(s *State) { s.Reserved = false })
		r.gate.permits.Release(activityPermits)
	})
}
