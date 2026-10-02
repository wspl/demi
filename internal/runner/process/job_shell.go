package process

import (
	"context"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runnerwire"
)

// JobCommands supplies the declared roots a job runs as builtins and their handler.
type JobCommands struct {
	// Context is the execution context every invocation names, also in DEMI_CONTEXT_ID.
	Context string
	// Roots is the root commands the job's manifest declares.
	Roots   []string
	Handler cmdsdk.Handler[commandwire.LocalInvocation]
}

// JobStart supplies what a job starts with. The context passed to JobShell.Start
// owns cancellation of the job and everything it runs.
type JobStart struct {
	Script string
	Cwd    string
	Env    map[string]string
	// Live says whether input is the job's live terminal rather than a finite body.
	Live     bool
	Commands *JobCommands
	// Edits records the files the job changes.
	Edits *cmdsdk.Recorder
}

// JobShell runs the runner's jobs without exposing its interpreter.
type JobShell interface {
	Start(context.Context, JobStart) (ShellJob, error)
	// BuiltinNames returns the shell's reserved root names as a set.
	BuiltinNames() map[string]struct{}
}

// ShellJob is one running job. Its owner must Cancel when abandoning it and
// always Wait; output must be consumed concurrently unless the job is cancelled.
type ShellJob interface {
	// Input is where the job's input goes.
	Input() chan<- Input
	// Output yields chunks until everything the job ran has finished.
	Output() <-chan OutputChunk
	// Signal cancels for a terminating signal and refuses other signals.
	Signal(runnerwire.Signal) error
	// Cancel ends the job and everything it runs.
	Cancel()
	// IsCancelled reports whether cancellation was requested.
	IsCancelled() bool
	// Wait joins all job work and returns its exit and optional last directory.
	// Cancellation cancels the job and still joins before returning.
	Wait(context.Context) (Exit, *string)
}
