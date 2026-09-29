package machines

import (
	"context"
	"errors"

	"github.com/wspl/demi/go/gates"
)

// ErrStopping means the manager is stopping and admits no work.
var ErrStopping = errors.New("the Cloud manager is stopping")

// An admission is the manager-wide gate (docs/architecture/concurrency.md §
// Machine manager): device operations share it, reconcile and shutdown take it
// whole. It is first in, first out, so an exclusive entrant waits for the
// operations in flight and every later request waits behind it.
type admission struct {
	gate *gates.ActivityGate
	// closed is done once the gate is closed.
	closed context.Context
	shut   context.CancelFunc
}

func newAdmission() *admission {
	closed, cancel := context.WithCancel(context.Background())
	return &admission{gate: gates.NewActivityGate(), closed: closed, shut: cancel}
}

// enter admits one device operation beside others. It refuses with
// [ErrStopping] once the gate is closed, waiting entrants included.
func (a *admission) enter(ctx context.Context) (*gates.Lease, error) {
	ctx, stop := a.until(ctx)
	defer stop()
	lease, err := a.gate.Enter(ctx, gates.Maintenance)
	if err != nil {
		return nil, a.refusal(err)
	}
	if a.closed.Err() != nil {
		lease.Release()
		return nil, ErrStopping
	}
	return lease, nil
}

// exclusive admits work that needs every device operation finished.
func (a *admission) exclusive(ctx context.Context) (*gates.Reservation, error) {
	ctx, stop := a.until(ctx)
	defer stop()
	reservation, err := a.gate.Reserve(ctx)
	if err != nil {
		return nil, a.refusal(err)
	}
	if a.closed.Err() != nil {
		reservation.Release()
		return nil, ErrStopping
	}
	return reservation, nil
}

// close refuses every entrant from now on, the waiting ones too, so no request
// accepted before shutdown can start a sandbox after the drain.
func (a *admission) close() {
	a.shut()
}

// until returns a context that ends when ctx does or the gate closes; the
// returned function releases it.
func (a *admission) until(ctx context.Context) (context.Context, func()) {
	ctx, cancel := context.WithCancel(ctx)
	stop := context.AfterFunc(a.closed, cancel)
	return ctx, func() {
		stop()
		cancel()
	}
}

// refusal names why a wait ended: the gate closed, or the caller's context.
func (a *admission) refusal(err error) error {
	if a.closed.Err() != nil {
		return ErrStopping
	}
	return err
}
