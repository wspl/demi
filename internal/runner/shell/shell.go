package shell

import (
	"context"

	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runner/shell/internal/engine"
	"mvdan.cc/sh/v3/interp"
)

// Shell runs each job in a fresh interpreter with its own state and owned work.
// A Shell may start concurrent jobs. Go's scheduler runs their interpreter work;
// there is no separate runtime or blocking pool to shut down.
type Shell struct{}

// New constructs a shell service. Each Start supplies the job's command handler,
// environment, edit recorder and cancellation context.
func New() *Shell { return &Shell{} }

// Start starts a fresh login shell. The context owns the job's lifetime.
// The caller consumes output concurrently and always calls Wait, including
// after cancellation, to join every interpreter task and child process.
func (s *Shell) Start(ctx context.Context, job process.JobStart) (process.ShellJob, error) {
	return engine.StartJob(ctx, job, nil)
}

// BuiltinNames returns a fresh set of reserved command roots for the dispatcher.
// System utilities are external commands and are not reserved shell builtins.
func (s *Shell) BuiltinNames() map[string]struct{} {
	names := map[string]struct{}{}
	for _, name := range interp.BuiltinNames() {
		names[name] = struct{}{}
	}
	return names
}

var _ process.JobShell = (*Shell)(nil)
