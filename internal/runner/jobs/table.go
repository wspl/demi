//revive:disable:unused-parameter API checkpoint retains parameter names; bodies follow after merge.

package jobs

import (
	"context"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// WorkKind distinguishes shell jobs from raw processes with independent IDs.
type WorkKind uint8

const (
	// ShellWork identifies a shell job.
	ShellWork WorkKind = iota
	// ProcessWork identifies a raw process.
	ProcessWork
)

// WorkID is a job's or raw process's ID, which the two kinds do not share.
type WorkID struct {
	Kind WorkKind
	ID   string
}

// Config supplies what every task of one connection shares.
// Output is a bounded queue of encoded runner frames owned by the connection.
// The caller owns Pipes, Directories and Shell and keeps them until Table.Close.
type Config struct {
	Output      chan<- []byte
	Directories *Directories
	Pipes       *process.PipeClient
	Shell       process.JobShell
	// Commands is absent where nothing makes execution contexts live.
	Commands *Commands
}

// Commands supplies the execution context and dispatcher for declared commands.
type Commands struct {
	Dispatcher   *Dispatcher
	Connection   *ConnectionHandle
	Installation *Installation
	Paths        ContextPaths
	Services     *cmdpkgs.ServiceHandle
	// Endpoint is the local endpoint command clients reach.
	Endpoint string
	// Home is the installation directory, DEMI_HOME.
	Home string
}

// TaskCommand selects shell execution or a raw process.
//
//sumtype:decl
type TaskCommand interface{ taskCommand() }

// ShellCommand runs a script with optional pipe input and output.
type ShellCommand struct {
	Script string
	Stdin  *runnerwire.PipeRef
	Stdout *runnerwire.PipeRef
	// Commands pins the manifest and command context, or is nil for no declarations.
	Commands *DeclaredCommands
}

func (*ShellCommand) taskCommand() { panic("not written: r-jobs") }

// DeclaredCommands names the manifest and backend command context of a job.
type DeclaredCommands struct {
	ManifestHash string
	Context      commandwire.CommandContext
}

// ProcessCommand runs one raw executable with its arguments.
type ProcessCommand struct {
	Command      string
	Args         []string
	ProcessGroup bool
}

func (*ProcessCommand) taskCommand() { panic("not written: r-jobs") }

// TaskSpec supplies a task's ID, absolute directory, environment and command.
type TaskSpec struct {
	ID      string
	Cwd     string
	Env     map[string]string
	Command TaskCommand
}

// Table owns a connection's jobs and raw processes, including setup and IO.
// Its methods are safe for concurrent use. Its owner must call Close.
type Table struct{}

// NewTable creates the connection's task owner. Cancelling ctx cancels all tasks;
// Close must still join them. The connection retains ownership of config's resources.
func NewTable(ctx context.Context, config Config) *Table { panic("not written: r-jobs") }

// Len returns the number of registered tasks, including completed tasks not collected.
func (t *Table) Len() int { panic("not written: r-jobs") }

// JobCount returns the number of registered shell jobs.
func (t *Table) JobCount() int { panic("not written: r-jobs") }

// Start registers the task before starting setup so subsequent input finds it.
// Task lifetime belongs to the table, not a request's context.
func (t *Table) Start(spec TaskSpec) error { panic("not written: r-jobs") }

// Input queues live stdin without blocking the connection. An overflowing queue
// cancels the task; bulk streams use independently flowing HTTP pipes.
func (t *Table) Input(id WorkID, bytes []byte) error { panic("not written: r-jobs") }

// EndInput queues EOF for the task's live input.
func (t *Table) EndInput(id WorkID) error { panic("not written: r-jobs") }

// Follow starts or stops output beyond each stream's initial view.
func (t *Table) Follow(id WorkID, follow bool) { panic("not written: r-jobs") }

// Signal delivers a signal without blocking; kill cancels the task.
func (t *Table) Signal(id WorkID, signal runnerwire.Signal) error { panic("not written: r-jobs") }

// Finished waits for the next task after its result has been sent, removing its
// entry. The boolean is false when no task runs; an interrupted wait returns an error.
func (t *Table) Finished(ctx context.Context) (WorkID, bool, error) { panic("not written: r-jobs") }

// Close cancels and joins every task, suppressing further results. Cancellation
// of ctx still initiates shutdown; a later Close can finish waiting. It is idempotent.
func (t *Table) Close(ctx context.Context) error { panic("not written: r-jobs") }

// FailureExit encodes the end of work that could not run, or failed before its
// status was known: no exit status and reason as its spawn error.
func FailureExit(work WorkID, reason string) ([]byte, error) {
	panic("not written: r-jobs")
}
