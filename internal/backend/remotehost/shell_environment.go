package remotehost

//revive:disable:unused-parameter
// API checkpoint: keep parameter names for callers until the bodies are implemented.

import (
	"context"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// HostAccess holds the product's file gate and admission through a job's publication.
// RunJob refuses a stale Host and calls job once with the admitted lifetime's context.
type HostAccess interface {
	RunJob(ctx context.Context, key host.Key, job func(context.Context) error) error
}

// CommandKeeper stores edit copies and whole output before publishing the command's end.
// Failure to keep output is the keeper's to record.
type CommandKeeper interface {
	Retain(context.Context, core.CommandID, []runnerwire.JobFileChange) ([]core.EditedFile, error)
	KeepOutput(context.Context, core.CommandID, host.WholeOutput) error
}

// ContextSource builds the command context for each job.
type ContextSource func(context.Context) (commandwire.CommandContext, error)

// EnvironmentOptions supplies a node's Host, commands, page feed and lifetime services.
type EnvironmentOptions struct {
	Host        *Host
	Commands    *CommandSelection
	Context     ContextSource
	Feed        host.PageFeed
	Numbers     host.Numbers
	Access      HostAccess
	Keeper      CommandKeeper
	InitialEnv  map[string]string
	OutputLimit int
	BinaryLimit int
}

// NewEnvironmentOptions supplies the standard output and binary budgets.
func NewEnvironmentOptions(h *Host, source ContextSource, feed host.PageFeed, numbers host.Numbers) EnvironmentOptions {
	panic("not written: b-remotehost")
}

// ShellEnvironmentFactory creates node environments against a fixed startup catalog.
type ShellEnvironmentFactory struct{ _ byte }

// NewShellEnvironmentFactory constructs the factory with its package catalog.
func NewShellEnvironmentFactory(catalog *CommandCatalog) *ShellEnvironmentFactory {
	panic("not written: b-remotehost")
}

// Create constructs an environment whose jobs run commands on options.Host.
func (f *ShellEnvironmentFactory) Create(commands *host.CommandSet, options EnvironmentOptions) (*ShellEnvironment, error) {
	panic("not written: b-remotehost")
}

// ShellEnvironment owns a node's shells on a Host behind a runner.
// DisposeAll cancels and joins every job before its owner releases the environment.
type ShellEnvironment struct{ _ byte }

// NewShellEnvironment constructs a node's environment.
func NewShellEnvironment(options EnvironmentOptions) *ShellEnvironment {
	panic("not written: b-remotehost")
}

// Exec starts a shell command and observes it for the requested window.
func (e *ShellEnvironment) Exec(ctx context.Context, request host.ExecRequest) (host.CommandStatus, error) {
	panic("not written: b-remotehost")
}

// Status returns the command's status and output since the previous look.
func (e *ShellEnvironment) Status(id core.CommandID) (host.CommandStatus, error) {
	panic("not written: b-remotehost")
}

// ReadOutput reads a command's whole kept output.
func (e *ShellEnvironment) ReadOutput(ctx context.Context, id core.CommandID) (host.WholeOutput, error) {
	panic("not written: b-remotehost")
}

// Write supplies live standard input to a running command.
func (e *ShellEnvironment) Write(ctx context.Context, id core.CommandID, bytes []byte) error {
	panic("not written: b-remotehost")
}

// Abort terminates a command and joins its output and edit publication.
func (e *ShellEnvironment) Abort(ctx context.Context, id core.CommandID) error {
	panic("not written: b-remotehost")
}

// PageViews returns the commands' current page views.
func (e *ShellEnvironment) PageViews() []host.PageView { panic("not written: b-remotehost") }

// ReleaseCommand disposes a command handle and reports whether it existed.
func (e *ShellEnvironment) ReleaseCommand(ctx context.Context, id core.CommandID) bool {
	panic("not written: b-remotehost")
}

// DisposeShell disposes a shell and its commands, reporting whether it existed.
func (e *ShellEnvironment) DisposeShell(ctx context.Context, id core.ShellID) bool {
	panic("not written: b-remotehost")
}

// DisposeAll cancels and joins every owned job and releases every shell.
func (e *ShellEnvironment) DisposeAll(ctx context.Context) error { panic("not written: b-remotehost") }

// OwnsShell reports whether the shell belongs to this environment.
func (e *ShellEnvironment) OwnsShell(id core.ShellID) bool { panic("not written: b-remotehost") }

// OwnsCommand reports whether the command belongs to this environment.
func (e *ShellEnvironment) OwnsCommand(id core.CommandID) bool { panic("not written: b-remotehost") }

// EditedFile builds a file's page record with stored copies for each edit segment.
func EditedFile(file runnerwire.JobFileChange, copies func(int) *core.EditCopies) core.EditedFile {
	panic("not written: b-remotehost")
}

var _ host.ShellEnvironment = (*ShellEnvironment)(nil)
