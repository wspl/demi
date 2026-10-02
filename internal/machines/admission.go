package machines

import (
	"context"
	"errors"
	"sync"
)

// ErrClosed means the Cloud manager is stopping.
var ErrClosed = errors.New("the Cloud manager is stopping")

type entrant struct {
	exclusive bool
	ready     chan struct{}
	admitted  bool
}

// Admission admits shared work in arrival order and gives reconcile and shutdown exclusive access.
// Its zero value is ready to use.
type Admission struct {
	mu                sync.Mutex
	queue             []*entrant
	active            int
	exclusive, closed bool
}

// Enter admits one device operation. The returned release must be called once.
func (a *Admission) Enter(ctx context.Context) (func(), error) { return a.acquire(ctx, false) }

// Exclusive waits for all earlier operations and holds back later entrants.
func (a *Admission) Exclusive(ctx context.Context) (func(), error) { return a.acquire(ctx, true) }
func (a *Admission) acquire(ctx context.Context, exclusive bool) (func(), error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	e := &entrant{exclusive: exclusive, ready: make(chan struct{})}
	a.mu.Lock()
	if a.closed {
		a.mu.Unlock()
		return nil, ErrClosed
	}
	a.queue = append(a.queue, e)
	a.advance()
	a.mu.Unlock()
	select {
	case <-e.ready:
	case <-ctx.Done():
	}
	a.mu.Lock()
	if !e.admitted {
		for i, v := range a.queue {
			if v == e {
				a.queue = append(a.queue[:i], a.queue[i+1:]...)
				break
			}
		}
		a.advance()
		closed := a.closed
		a.mu.Unlock()
		if closed {
			return nil, ErrClosed
		}
		return nil, ctx.Err()
	}
	a.mu.Unlock()
	var once sync.Once
	return func() {
		once.Do(func() {
			a.mu.Lock()
			a.active--
			if exclusive {
				a.exclusive = false
			}
			a.advance()
			a.mu.Unlock()
		})
	}, nil
}

// advance grants queued Cloud operations without passing an exclusive waiter.
func (a *Admission) advance() {
	for !a.closed && !a.exclusive && len(a.queue) > 0 {
		e := a.queue[0]
		if e.exclusive && a.active != 0 {
			return
		}
		a.queue = a.queue[1:]
		a.active++
		a.exclusive = e.exclusive
		e.admitted = true
		close(e.ready)
	}
}

// Close refuses waiting and future work; admitted work retains its permits.
func (a *Admission) Close() {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.closed = true
	for _, e := range a.queue {
		close(e.ready)
	}
	a.queue = nil
}
