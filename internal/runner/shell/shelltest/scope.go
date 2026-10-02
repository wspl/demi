package shelltest

//revive:disable:unused-parameter // API checkpoint: parameter names document the boundary.

import (
	"context"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runner/process"
)

// Scope owns a test shell's interpreter tasks, child processes and IO adapters.
// Its owner must Cancel and Finish at cleanup. Commands and Edits must be set
// before starting work and must not change while the scope is in use.
type Scope struct {
	Commands *process.JobCommands
	Edits    *cmdsdk.Recorder
}

// NewScope creates an owned scope whose lifetime is bounded by ctx.
func NewScope(ctx context.Context, commands *process.JobCommands) *Scope {
	panic("not written: r-shell")
}

// Cancel requests cancellation of all work owned by the scope.
func (s *Scope) Cancel() { panic("not written: r-shell") }

// Finish joins all owned work. Cancellation cancels the scope and still joins
// before returning. Call Cancel first when abandoning unfinished work.
func (s *Scope) Finish(ctx context.Context) { panic("not written: r-shell") }

// Activity returns the event-based observation of the scope's interpreter work.
func (s *Scope) Activity() *Activity { panic("not written: r-shell") }

// Start starts a login shell in this scope. Commands and Edits come from the
// scope. The returned job has the same
// output consumption and Wait obligations as process.JobShell.Start.
func (s *Scope) Start(ctx context.Context, script, cwd string, env map[string]string, live bool) (process.ShellJob, error) {
	panic("not written: r-shell")
}

// Activity observes interruptible IO waits and interpreter cancellation checks.
// Observations are safe while the shell runs; waits use events, not polling.
type Activity struct{}

// Waiting returns the number of interpreter units currently waiting for IO.
func (a *Activity) Waiting() int { panic("not written: r-shell") }

// Checks returns the number of interpreter cancellation checks so far.
func (a *Activity) Checks() uint64 { panic("not written: r-shell") }

// WaitWaiting waits until at least one interpreter unit is waiting for IO.
func (a *Activity) WaitWaiting(ctx context.Context) error { panic("not written: r-shell") }

// WaitBlockedOrChecks waits for a unit waiting for IO or at least minChecks
// cancellation checks. A test uses Checks plus its desired progress count to
// observe either blocked input or a running loop before cancelling the job.
func (a *Activity) WaitBlockedOrChecks(ctx context.Context, minChecks uint64) error {
	panic("not written: r-shell")
}
