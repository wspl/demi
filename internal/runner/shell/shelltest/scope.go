package shelltest

import (
	"context"
	"sync"

	"github.com/wspl/demi/internal/commandsdk"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runner/shell/internal/engine"
)

// Scope owns a test shell's interpreter tasks, child processes and IO adapters.
// Its owner must Cancel and Finish at cleanup.
type Scope struct {
	// Commands must be set before work starts and remain unchanged while in use.
	Commands *process.JobCommands
	// Edits must be set before work starts and remain unchanged while in use.
	Edits    *commandsdk.Recorder
	ctx      context.Context
	cancel   context.CancelFunc
	work     sync.WaitGroup
	activity Activity
}

// NewScope creates an owned scope whose lifetime is bounded by ctx.
func NewScope(ctx context.Context, commands *process.JobCommands) *Scope {
	ctx, cancel := context.WithCancel(ctx)
	return &Scope{ctx: ctx, cancel: cancel, Commands: commands}
}

// Cancel requests cancellation of all work owned by the scope.
func (s *Scope) Cancel() {
	s.cancel()
}

// Finish joins all owned work. Cancellation cancels the scope and still joins
// before returning. Call Cancel first when abandoning unfinished work.
func (s *Scope) Finish(ctx context.Context) {
	stopped := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		s.cancel()
		close(stopped)
	})
	s.work.Wait()
	s.cancel()
	if !stop() {
		<-stopped
	}
}

// Activity returns the event-based observation of the scope's interpreter work.
func (s *Scope) Activity() *Activity {
	return &s.activity
}

// Start starts a login shell in this scope. Commands and Edits come from the
// scope. The returned job has the same output consumption and Wait obligations
// as process.JobShell.Start.
func (s *Scope) Start(
	ctx context.Context,
	script, cwd string,
	env map[string]string,
	live bool,
) (process.ShellJob, error) {
	ctx, finish := s.execution(ctx)
	job, err := engine.StartJob(
		ctx,
		process.JobStart{Script: script, Cwd: cwd, Env: env, Live: live, Commands: s.Commands, Edits: s.Edits},
		observer{&s.activity},
	)
	if err != nil {
		finish()
		return nil, err
	}
	go func() {
		defer finish()
		_, _, _ = job.Wait(context.Background())
	}()
	return job, nil
}

// execution links the caller's cancellation to a scope-owned execution and joins
// its cancellation callback before releasing the scope's work registration.
func (s *Scope) execution(ctx context.Context) (context.Context, func()) {
	owner, cancel := context.WithCancel(s.ctx)
	stopped := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		cancel()
		close(stopped)
	})
	if err := ctx.Err(); err != nil {
		cancel()
	}
	s.work.Add(1)
	return owner, func() {
		cancel()
		if !stop() {
			<-stopped
		}
		s.work.Done()
	}
}

// Activity observes interruptible IO waits and interpreter cancellation checks.
// Observations are safe while the shell runs; waits use events, not polling.
type Activity struct {
	mu      sync.Mutex
	waiting int
	checks  uint64
	changed chan struct{}
}

// Waiting returns the number of interpreter units currently waiting for IO.
func (a *Activity) Waiting() int {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.waiting
}

// Checks returns the number of interpreter cancellation checks so far.
func (a *Activity) Checks() uint64 {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.checks
}

// WaitWaiting waits until at least one interpreter unit is waiting for IO.
func (a *Activity) WaitWaiting(ctx context.Context) error {
	return a.wait(ctx, 0)
}

// WaitBlockedOrChecks waits for a unit waiting for IO or at least minChecks
// cancellation checks. A test uses Checks plus its desired progress count to
// observe either blocked input or a running loop before cancelling the job.
func (a *Activity) WaitBlockedOrChecks(ctx context.Context, minChecks uint64) error {
	return a.wait(ctx, minChecks)
}

func (a *Activity) wait(ctx context.Context, minChecks uint64) error {
	for {
		a.mu.Lock()
		ready := a.waiting > 0 || (minChecks > 0 && a.checks >= minChecks)
		if a.changed == nil {
			a.changed = make(chan struct{})
		}
		changed := a.changed
		a.mu.Unlock()
		if ready {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-changed:
		}
	}
}

type observer struct{ activity *Activity }

// Check reports interpreter progress to the scope activity.
func (o observer) Check() {
	o.update(0, 1)
}

// Waiting reports changes in interruptible IO waits to the scope activity.
func (o observer) Waiting(delta int) {
	o.update(delta, 0)
}

func (o observer) update(waiting int, checks uint64) {
	a := o.activity
	a.mu.Lock()
	a.waiting += waiting
	a.checks += checks
	changed := a.changed
	a.changed = nil
	a.mu.Unlock()
	if changed != nil {
		close(changed)
	}
}
