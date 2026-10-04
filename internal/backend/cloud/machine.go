package cloud

import (
	"context"
	"errors"
	"log/slog"
	"runtime/debug"
	"sync"
	"time"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapiproto"
)

// Cloud is the user's machine and owned lifecycle work. New binds it to the
// shard's mutex. All methods acquire that mutex themselves, never across IO.
type Cloud struct {
	mu      *sync.Mutex // The shard's mutex protects machine state and worker admission.
	ctx     context.Context
	cancel  context.CancelFunc
	machine *machine
	stopped bool
	workers sync.WaitGroup
	closing *transition
	records records // Immutable storage boundary; nil selects Shard.Control.
}
type machine struct {
	device     database.DeviceRecord
	gate       *gates.Activity
	phase      webapiproto.CloudState
	permit     *Permit
	transition *transition
	reset      *transition
	retirement *transition
	operation  *database.ManagedOperation
	failure    *string
	deaths     []time.Time
	started    time.Time
	checkpoint time.Time
	schedules  context.CancelFunc
	marks      pagesync.UserMarks
}
type transition struct {
	done chan struct{}
	err  error
}

// New creates a Cloud with its shard's lifetime context and mutex. Close must
// join owned work even after ctx is canceled.
func New(ctx context.Context, mu *sync.Mutex) *Cloud {
	life, cancel := context.WithCancel(ctx)
	return &Cloud{mu: mu, ctx: life, cancel: cancel}
}

// Runs reports whether device's machine runs, which a new expose needs.
func (c *Cloud) Runs(device webapiproto.DeviceID) bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.machine != nil && c.machine.device.ID == device && c.machine.phase == webapiproto.CloudStateRunning
}

// Stop refuses new admissions and cancels schedules without waiting. Close
// joins schedules and any retirement already committed.
func (c *Cloud) Stop() {
	c.mu.Lock()
	c.stopped = true
	var cancel context.CancelFunc
	if c.machine != nil {
		cancel = c.machine.schedules
	}
	c.mu.Unlock()
	c.cancel()
	if cancel != nil {
		cancel()
	}
}

// Admission holds the Cloud running; each Host operation takes PerOperation.
// The acquiring owner defers Release.
type Admission struct {
	// Device is the admitted Cloud's identity.
	Device webapiproto.DeviceID
	// PerOperation acquires a separate lease for each Host operation.
	PerOperation remotehost.Admission
	held         *gates.Lease
}

// Release lets this admission go. It is idempotent and does not wait.
func (a *Admission) Release() {
	a.held.Release()
}

// Admit wakes a stopped Cloud or joins boot, recovery or reset. Request
// cancellation ends the wait, not an owned transition. The caller owns device.
func Admit(ctx context.Context, shard Shard, device database.DeviceRecord) (*Admission, error) {
	m, err := loadMachine(ctx, shard, device)
	if err != nil {
		return nil, storageFailed(err)
	}
	c := shard.Cloud()
	for {
		c.mu.Lock()
		stopped := c.stopped || c.ctx.Err() != nil
		reset := m.reset
		if reset == nil {
			reset = m.retirement
		}
		c.mu.Unlock()
		if stopped {
			return nil, &Error{Kind: Closed}
		}
		if reset != nil {
			if err := reset.wait(ctx); err != nil {
				return nil, err
			}
			continue
		}
		lease, err := m.gate.Enter(ctx, gates.Demand)
		if err != nil {
			return nil, err
		}
		ready, err := ensureRunning(ctx, shard, m)
		if err != nil || !ready {
			lease.Release()
			if err != nil {
				return nil, err
			}
			continue
		}
		return &Admission{
			Device: device.ID,
			held:   lease,
			PerOperation: func() (*gates.Lease, error) {
				return c.admitOperation(m)
			},
		}, nil
	}
}

// admitOperation acquires a lease then rechecks the Cloud phase so reset cannot
// win the decision while an operation escapes its admission boundary.
func (c *Cloud) admitOperation(m *machine) (*gates.Lease, error) {
	c.mu.Lock()
	phase, retiring := m.phase, m.retirement != nil
	c.mu.Unlock()
	if phase != webapiproto.CloudStateRunning {
		return nil, &host.Error{Kind: host.Unavailable, Message: "Cloud is not accepting operations"}
	}
	if retiring {
		return nil, &host.Error{Kind: host.Unavailable, Message: "Cloud is changing state"}
	}
	lease := m.gate.TryEnter(gates.Demand)
	if lease == nil {
		return nil, &host.Error{Kind: host.Unavailable, Message: "Cloud is changing state"}
	}
	c.mu.Lock()
	running := m.phase == webapiproto.CloudStateRunning && m.retirement == nil
	c.mu.Unlock()
	if !running {
		lease.Release()
		return nil, &host.Error{Kind: host.Unavailable, Message: "Cloud is not accepting operations"}
	}
	return lease, nil
}

// Died records an unsolicited runtime loss. Saving and resetting ignore it.
func Died(ctx context.Context, shard Shard, device webapiproto.DeviceID) error {
	c := shard.Cloud()
	tuning := shard.CloudServices().Tuning
	c.mu.Lock()
	m := c.machine
	if m == nil || m.device.ID != device ||
		(m.phase != webapiproto.CloudStateRunning && m.phase != webapiproto.CloudStateBooting) {
		c.mu.Unlock()
		return nil
	}
	now := time.Now()
	m.deaths = append(m.deaths, now)
	for len(m.deaths) > 0 && now.Sub(m.deaths[0]) >= tuning.CrashLoopWindow {
		m.deaths = m.deaths[1:]
	}
	if m.phase == webapiproto.CloudStateBooting {
		c.mu.Unlock()
		return nil
	}
	cancel, permit := m.schedules, m.permit
	m.schedules = nil
	m.permit = nil
	m.phase = webapiproto.CloudStateOff
	c.mu.Unlock()
	if cancel != nil {
		cancel()
	}
	if permit != nil {
		permit.Release()
	}
	m.mark()
	shard.Devices().Disconnect(device, "the Cloud's sandbox stopped")
	return shard.CloudStopped(ctx, device)
}

// Close stops admission, joins work, then saves a running machine if its gate
// is free. A still-held machine is left to the manager's final reconciliation.
func Close(ctx context.Context, shard Shard) error {
	c := shard.Cloud()
	c.Stop()
	c.mu.Lock()
	if t := c.closing; t != nil {
		c.mu.Unlock()
		return t.wait(ctx)
	}
	t := &transition{done: make(chan struct{})}
	c.closing = t
	c.mu.Unlock()
	// Stop fenced registration; durable transitions finish with cleanup contexts.
	c.workers.Wait()
	c.mu.Lock()
	m := c.machine
	c.mu.Unlock()
	if m != nil {
		if reserved := m.gate.TryReserve(); reserved != nil {
			t.err = hibernate(ctx, shard, m)
			reserved.Release()
		}
	}
	c.workers.Wait()
	// Close gives back the Cloud's remaining capacity permit, also for a machine
	// it leaves running for the manager's final reconciliation.
	c.mu.Lock()
	var permit *Permit
	if m != nil {
		permit = m.permit
		m.permit = nil
	}
	c.mu.Unlock()
	if permit != nil {
		permit.Release()
	}
	close(t.done)
	return t.err
}

// loadMachine publishes the first Cloud machine after its durable reset read.
func loadMachine(ctx context.Context, s Shard, device database.DeviceRecord) (*machine, error) {
	c := s.Cloud()
	c.mu.Lock()
	m := c.machine
	c.mu.Unlock()
	if m != nil {
		return m, nil
	}
	operation, found, err := cloudRecords(s).LatestManagedOperation(ctx, device.ID)
	if err != nil {
		return nil, err
	}
	marks := s.Marks()
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.machine == nil {
		c.machine = &machine{
			device: device,
			gate:   gates.NewActivity(nil),
			phase:  webapiproto.CloudStateOff,
			marks:  marks,
		}
		if found {
			c.machine.operation = &operation
		}
	}
	return c.machine, nil
}

// wait joins one Cloud transition without granting its waiter cancellation ownership.
func (t *transition) wait(ctx context.Context) error {
	select {
	case <-t.done:
		return t.err
	case <-ctx.Done():
		return ctx.Err()
	}
}

// mark publishes a Cloud change only after its shard mutex has been released.
func (m *machine) mark() {
	m.marks.Mark(pagesync.Part{Kind: pagesync.Cloud})
}

// ensureRunning atomically chooses or joins the machine transition.
func ensureRunning(ctx context.Context, s Shard, m *machine) (bool, error) {
	c := s.Cloud()
	services := s.CloudServices()
	for {
		online := s.Devices().Online(m.device.ID)
		c.mu.Lock()
		if c.stopped || c.ctx.Err() != nil {
			c.mu.Unlock()
			return false, &Error{Kind: Closed}
		}
		if m.transition != nil {
			t := m.transition
			c.mu.Unlock()
			if err := t.wait(ctx); err != nil {
				return false, err
			}
			continue
		}
		if m.phase == webapiproto.CloudStateResetting || m.retirement != nil {
			c.mu.Unlock()
			return false, nil
		}
		if m.phase == webapiproto.CloudStateRunning && online {
			c.mu.Unlock()
			return true, nil
		}
		recovering := m.phase == webapiproto.CloudStateRunning
		if !recovering {
			if err := prepareBootLocked(c, m, services); err != nil {
				return false, err
			}
		}
		t := &transition{done: make(chan struct{})}
		m.transition = t
		c.workers.Add(1)
		c.mu.Unlock()
		if !recovering {
			m.mark()
		}
		go func() {
			defer c.workers.Done()
			completeBootTransition(c.ctx, s, c, m, t, recovering)
		}()
		if err := t.wait(ctx); err != nil {
			return false, err
		}
	}
}

// runTransition turns a panic in work into a failed transition, so waiters get an answer.
func runTransition(ctx context.Context, work func(context.Context) error) (err error) {
	defer func() {
		if recovered := recover(); recovered != nil {
			slog.Error("cloud transition panicked", "panic", recovered, "stack", string(debug.Stack()))
			//nolint:staticcheck // Product text, shown to the user as it is.
			err = failed(errors.New("A transition of the Cloud ended without an answer"))
		}
	}()
	return work(ctx)
}

// prepareBootLocked checks crash history and takes capacity with the shard mutex held.
// On failure it releases the mutex before constructing the error.
func prepareBootLocked(c *Cloud, m *machine, services *Services) error {
	if m.phase != webapiproto.CloudStateOff {
		c.mu.Unlock()
		return failed(errors.New("Cloud is changing state"))
	}
	count := uint32(0)
	for _, at := range m.deaths {
		if time.Since(at) < services.Tuning.CrashLoopWindow {
			count++
		}
	}
	if count >= services.Tuning.CrashLoopDeaths {
		c.mu.Unlock()
		return &Error{Kind: CrashLoop}
	}
	// Lock order is shard then capacity; capacity never calls into a shard.
	permit := services.Capacity.TryTake()
	if permit == nil {
		c.mu.Unlock()
		return &Error{Kind: AtCapacity}
	}
	m.permit = permit
	m.phase = webapiproto.CloudStateBooting
	return nil
}

// completeBootTransition publishes readiness before waking callers waiting on the transition.
func completeBootTransition(ctx context.Context, s Shard, c *Cloud, m *machine, t *transition, recovering bool) {
	err := runTransition(context.WithoutCancel(ctx), func(ctx context.Context) error {
		if recovering {
			return recoverMachine(ctx, s, m)
		}
		return boot(ctx, s, m)
	})
	finishBoot(s, m, err)
	c.mu.Lock()
	m.transition = nil
	c.mu.Unlock()
	t.err = err
	close(t.done)
}
