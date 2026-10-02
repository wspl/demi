package shell

//revive:disable:unused-parameter // API checkpoint: parameter names document the boundary.

import (
	"context"

	"github.com/wspl/demi/internal/runner/process"
)

// Shell runs each job in a fresh interpreter with its own state and owned work.
// A Shell may start concurrent jobs. Go's scheduler runs their interpreter work;
// there is no separate runtime or blocking pool to shut down.
type Shell struct{}

// New constructs a shell service. Each Start supplies the job's command handler,
// environment, edit recorder and cancellation context.
func New() *Shell { panic("not written: r-shell") }

// Start starts a fresh login shell. The context owns the job's lifetime.
// The caller consumes output concurrently and always calls Wait, including
// after cancellation, to join every interpreter task and child process.
func (s *Shell) Start(ctx context.Context, job process.JobStart) (process.ShellJob, error) {
	panic("not written: r-shell")
}

// BuiltinNames returns a fresh set of reserved command roots for the dispatcher.
// System utilities are external commands and are not reserved shell builtins.
func (s *Shell) BuiltinNames() map[string]struct{} { panic("not written: r-shell") }

var _ process.JobShell = (*Shell)(nil)
