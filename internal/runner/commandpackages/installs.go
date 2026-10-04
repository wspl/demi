package commandpackages

import (
	"context"
	"math/bits"
	"slices"
	"sync"

	"github.com/wspl/demi/internal/runnerproto"
)

// Installs tracks downloads and unpacking in progress. Its zero value is usable.
// Share a pointer rather than copying it after first use.
type Installs struct {
	mu      sync.Mutex
	list    []*installing
	changed chan struct{}
	closed  bool
}

// notificationLocked requires the install mutex, which also protects channel replacement.
func (i *Installs) notificationLocked() chan struct{} {
	if i.changed == nil {
		i.changed = make(chan struct{})
	}
	return i.changed
}

// notifyLocked requires the install mutex so every observer sees one channel generation.
func (i *Installs) notifyLocked() {
	close(i.notificationLocked())
	i.changed = make(chan struct{})
}

// Subscribe observes the current installs and subsequent changes.
func (i *Installs) Subscribe() *InstallsSubscription {
	i.mu.Lock()
	defer i.mu.Unlock()
	return &InstallsSubscription{installs: i, seen: i.notificationLocked()}
}

// Close ends reporting and wakes subscriptions after the owner has joined its installs.
// Repeated calls do nothing.
func (i *Installs) Close() {
	i.mu.Lock()
	defer i.mu.Unlock()
	if !i.closed {
		i.closed = true
		close(i.notificationLocked())
	}
}

// InstallsSubscription observes installation snapshots. Each subscription has its own cursor.
// One goroutine owns a subscription; use Subscribe for another observer.
type InstallsSubscription struct {
	installs *Installs
	seen     <-chan struct{}
}

// Current reads and marks the current list seen, keeping the oldest installs
// when there are more than a wire message can carry.
func (r *InstallsSubscription) Current() []runnerproto.Install {
	i := r.installs
	i.mu.Lock()
	defer i.mu.Unlock()
	r.seen = i.notificationLocked()
	result := make([]runnerproto.Install, 0, min(len(i.list), runnerproto.MaxInstalls))
	for _, item := range i.list[:min(len(i.list), runnerproto.MaxInstalls)] {
		result = append(result, item.value)
	}
	return result
}

// Changed waits past the last observed list. It returns false when reporting ends,
// or an error when ctx is cancelled.
func (r *InstallsSubscription) Changed(ctx context.Context) (bool, error) {
	i := r.installs
	i.mu.Lock()
	if r.seen != i.notificationLocked() {
		r.seen = i.changed
		i.mu.Unlock()
		return true, nil
	}
	closed := i.closed
	i.mu.Unlock()
	if closed {
		return false, nil
	}
	select {
	case <-ctx.Done():
		return false, ctx.Err()
	case <-r.seen:
		i.mu.Lock()
		r.seen = i.notificationLocked()
		closed = i.closed
		i.mu.Unlock()
		return !closed, nil
	}
}

type installing struct {
	installs *Installs
	value    runnerproto.Install
}

func (i *Installs) start(wanted Wanted) *installing {
	item := &installing{
		installs: i,
		value: runnerproto.Install{
			Package: wanted.Package,
			Name:    wanted.Name,
			Version: wanted.Version,
			Phase:   runnerproto.InstallPhaseDownload,
			Total:   wanted.Artifact.Size,
		},
	}
	i.mu.Lock()
	defer i.mu.Unlock()
	i.list = append(i.list, item)
	i.notifyLocked()
	return item
}

func (i *installing) close() {
	owner := i.installs
	owner.mu.Lock()
	defer owner.mu.Unlock()
	owner.list = slices.DeleteFunc(owner.list, func(item *installing) bool { return item == i })
	owner.notifyLocked()
}

func (i *installing) downloaded(done uint64) {
	owner := i.installs
	owner.mu.Lock()
	defer owner.mu.Unlock()
	done = min(done, i.value.Total)
	// Divide a full-width product: progress must not overflow for a large artifact.
	hundredth := func(n uint64) uint64 {
		hi, lo := bits.Mul64(n, 100)
		q, _ := bits.Div64(hi, lo, i.value.Total)
		return q
	}
	if done != i.value.Done && (done == i.value.Total || hundredth(done) > hundredth(i.value.Done)) {
		i.value.Done = done
		owner.notifyLocked()
	}
}

func (i *installing) unpacking() {
	owner := i.installs
	owner.mu.Lock()
	defer owner.mu.Unlock()
	i.value.Phase = runnerproto.InstallPhaseUnpack
	i.value.Done = i.value.Total
	owner.notifyLocked()
}
