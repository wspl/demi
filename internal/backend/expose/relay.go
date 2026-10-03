package expose

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"sync"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// Relay refusals are translated to HTTP statuses by the edge.
var (
	// ErrRelayNotFound means the expose is missing or expired.
	ErrRelayNotFound = errors.New("no such expose")
	// ErrRelayDeviceOffline means the exposed device has no live connection.
	ErrRelayDeviceOffline = errors.New("the device is offline")
	// ErrLimit means the expose has reached its connection limit.
	ErrLimit = errors.New("the expose is at its connection limit")
	// ErrRemoved means the expose ended during admission or forwarding.
	ErrRemoved = errors.New("the expose was removed")
)

// UnreachableError carries the runner's connect failure code.
type UnreachableError struct{ Code string }

// Error returns the service connection failure.
func (e *UnreachableError) Error() string { return "the service is unreachable (" + e.Code + ")" }

// Exposes tracks a user's live connections. Its zero value is ready to use.
// The shard stops admission, closes this component, and joins its Relay callers
// before discarding it. Each admitted lease must be released even after EndAll.
type Exposes struct {
	// mu protects admission counts, lifecycle and worker registration only.
	mu      sync.Mutex
	live    map[webapi.ExposeID]*liveExpose
	workers map[*expiryWorker]struct{}
	closed  bool
}

type liveExpose struct {
	connections int
	ended       context.Context
	end         context.CancelFunc
	expiry      *expiryWorker
}

type expiryWorker struct {
	cancel context.CancelFunc
	done   chan struct{}
}

// RelayAdmission holds one connection's place until Release or Relay returns.
// A caller defers Release immediately, including on device connect failure.
type RelayAdmission struct {
	exposes *Exposes
	id      webapi.ExposeID
	live    *liveExpose
	record  database.ExposeRecord
	once    sync.Once
}

func (e *Exposes) register(id webapi.ExposeID) (*RelayAdmission, error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.closed {
		return nil, ErrRemoved
	}
	if e.live == nil {
		e.live = make(map[webapi.ExposeID]*liveExpose)
	}
	live := e.live[id]
	if live == nil {
		ctx, cancel := context.WithCancel(context.Background())
		live = &liveExpose{ended: ctx, end: cancel}
		e.live[id] = live
	}
	if live.connections == 64 {
		return nil, ErrLimit
	}
	live.connections++
	return &RelayAdmission{exposes: e, id: id, live: live}, nil
}

// End interrupts all connections for the destroyed exposes.
func (e *Exposes) End(ids []webapi.ExposeID) {
	e.mu.Lock()
	var ends []context.CancelFunc
	for _, id := range ids {
		if live := e.live[id]; live != nil {
			ends = append(ends, live.end)
		}
	}
	e.mu.Unlock()
	for _, end := range ends {
		end()
	}
}

// EndAll interrupts every connection, including admissions waiting on storage.
func (e *Exposes) EndAll() {
	e.mu.Lock()
	ends := make([]context.CancelFunc, 0, len(e.live))
	for _, live := range e.live {
		ends = append(ends, live.end)
	}
	e.mu.Unlock()
	for _, end := range ends {
		end()
	}
}

// Close stops admission, cancels expiry workers and waits for them. Relay
// callers are owned and joined by the shard; Close interrupts them via Ending.
func (e *Exposes) Close(ctx context.Context) error {
	e.mu.Lock()
	e.closed = true
	workers := make([]*expiryWorker, 0, len(e.workers))
	for worker := range e.workers {
		workers = append(workers, worker)
	}
	e.mu.Unlock()
	e.EndAll()
	for _, worker := range workers {
		worker.cancel()
	}
	for _, worker := range workers {
		select {
		case <-worker.done:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	return nil
}

// Active returns outstanding admissions, including revoked but unreleased leases.
func (e *Exposes) Active() int {
	e.mu.Lock()
	defer e.mu.Unlock()
	count := 0
	for _, live := range e.live {
		count += live.connections
	}
	return count
}

// Record returns the admitted record by value.
func (a *RelayAdmission) Record() database.ExposeRecord { return a.record }

// Ending closes once the expose ends. Device connection attempts also observe it.
func (a *RelayAdmission) Ending() <-chan struct{} { return a.live.ended.Done() }

// Release frees this admission, once, and cancels the last connection's expiry watch.
// Close joins canceled workers; Release never waits for storage or a worker.
func (a *RelayAdmission) Release() {
	a.once.Do(func() {
		e := a.exposes
		e.mu.Lock()
		a.live.connections--
		last := a.live.connections == 0
		worker := a.live.expiry
		if last {
			delete(e.live, a.id)
		}
		e.mu.Unlock()
		if last {
			a.live.end()
			if worker != nil {
				worker.cancel()
			}
		}
	})
}

// AdmitRelay admits a connection up to its device. The shard opens the device
// stream next, and releases admission on every failure to do so.
func AdmitRelay(ctx context.Context, shard ExposeShard, id webapi.ExposeID) (*RelayAdmission, error) {
	admission, err := shard.Exposes().register(id)
	if err != nil {
		return nil, err
	}
	record, err := owned(ctx, shard, id)
	if err == nil && record == nil {
		err = ErrRelayNotFound
	}
	if err == nil {
		var expired bool
		expired, err = destroyIfExpired(ctx, shard, *record)
		if err == nil && expired {
			err = ErrRelayNotFound
		}
	}
	if err == nil && admission.live.ended.Err() != nil {
		err = ErrRemoved
	}
	if err != nil {
		admission.Release()
		return nil, err
	}
	admission.record = *record
	return admission, nil
}

// Relay holds admission until released, cancellation, or expose destruction.
// Its caller owns this blocking call (and joins it if run in a goroutine).
// end closes the network stream and runs exactly once before admission is freed.
// The caller must not Release concurrently with Relay.
func (a *RelayAdmission) Relay(ctx context.Context, shard ExposeShard, released <-chan struct{}, end func()) {
	defer a.Release()
	defer end()
	a.watch(shard)
	select {
	case <-released:
	case <-a.Ending():
	case <-ctx.Done():
	}
}

func (a *RelayAdmission) watch(shard ExposeShard) {
	e := a.exposes
	e.mu.Lock()
	if e.closed || a.live.expiry != nil || a.live.connections == 0 {
		e.mu.Unlock()
		return
	}
	ctx, cancel := context.WithCancel(context.Background())
	worker := &expiryWorker{cancel: cancel, done: make(chan struct{})}
	a.live.expiry = worker
	if e.workers == nil {
		e.workers = make(map[*expiryWorker]struct{})
	}
	e.workers[worker] = struct{}{}
	e.mu.Unlock()
	go func() {
		defer func() {
			cancel()
			// All work and resource cleanup finish before relinquishing ownership.
			e.mu.Lock()
			delete(e.workers, worker)
			e.mu.Unlock()
			close(worker.done)
		}()
		expireWhenDue(ctx, shard, a.record)
	}()
}

func expireWhenDue(ctx context.Context, shard ExposeShard, record database.ExposeRecord) {
	for {
		if err := FirstExpiry(ctx, shard.Clock(), record.ExpiresAt); err != nil {
			if !errors.Is(err, context.Canceled) {
				slog.ErrorContext(ctx, "an expired expose could not be destroyed: "+err.Error(), "expose", record.ID)
			}
			return
		}
		next, err := owned(ctx, shard, record.ID)
		if err == nil && next == nil {
			return
		}
		if err == nil {
			var expired bool
			expired, err = destroyIfExpired(ctx, shard, *next)
			if err == nil && expired {
				return
			}
		}
		if err != nil {
			if !errors.Is(err, context.Canceled) {
				slog.ErrorContext(ctx, "an expired expose could not be destroyed: "+err.Error(), "expose", record.ID)
			}
			return
		}
		record = *next
	}
}

// FirstExpiry waits until the earliest expiry, or until ctx is canceled.
func FirstExpiry(ctx context.Context, clock core.Clock, first core.Timestamp) error {
	now, err := clock.Now().Time()
	if err != nil {
		return fmt.Errorf("expose clock: %w", err)
	}
	at, err := first.Time()
	if err != nil {
		return fmt.Errorf("expose expiry: %w", err)
	}
	timer := time.NewTimer(max(at.Sub(now), 0))
	defer timer.Stop()
	select {
	case <-timer.C:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}
