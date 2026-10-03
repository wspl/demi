package remotehost

import (
	"context"
	"log/slog"
	"math"
	"sync"
	"time"

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
	// Host is the runner Host on which jobs execute.
	Host *Host
	// Commands pins the commands available to jobs.
	Commands *CommandSelection
	// Context builds the command context for each job.
	Context ContextSource
	// Feed publishes command records and watching decisions.
	Feed host.PageFeed
	// Numbers allocates shell and command sequence numbers.
	Numbers host.Numbers
	// Access holds the product admission through job publication.
	Access HostAccess
	// Keeper retains file edits and whole command output.
	Keeper CommandKeeper
	// InitialEnv overlays variables on every new shell.
	InitialEnv map[string]string
	// OutputLimit bounds the returned text output bytes.
	OutputLimit int
	// BinaryLimit bounds retained binary stdout bytes.
	BinaryLimit int
}

// NewEnvironmentOptions supplies the standard output and binary budgets.
func NewEnvironmentOptions(h *Host, source ContextSource, feed host.PageFeed, numbers host.Numbers) EnvironmentOptions {
	return EnvironmentOptions{
		Host:        h,
		Context:     source,
		Feed:        feed,
		Numbers:     numbers,
		InitialEnv:  map[string]string{},
		OutputLimit: host.DefaultOutputLimitBytes,
		BinaryLimit: host.DefaultBinaryLimitBytes,
	}
}

// ShellEnvironmentFactory creates node environments against a fixed startup catalog.
type ShellEnvironmentFactory struct {
	catalog *CommandCatalog
}

// NewShellEnvironmentFactory constructs the factory with its package catalog.
func NewShellEnvironmentFactory(catalog *CommandCatalog) *ShellEnvironmentFactory {
	return &ShellEnvironmentFactory{catalog: catalog}
}

// Create constructs an environment whose jobs run commands on options.Host.
func (f *ShellEnvironmentFactory) Create(
	commands *host.CommandSet,
	options EnvironmentOptions,
) (*ShellEnvironment, error) {
	selection, err := f.catalog.Select(commands)
	if err != nil {
		return nil, err
	}
	options.Commands = selection
	return NewShellEnvironment(options), nil
}

// ShellEnvironment owns a node's shells on a Host behind a runner.
// DisposeAll cancels and joins every job before its owner releases the environment.
type ShellEnvironment struct {
	options                  EnvironmentOptions
	tasks                    sync.WaitGroup
	mu                       sync.Mutex // Protects shell reservations, record registration and running owners.
	shells                   map[core.ShellID]*shellState
	records                  map[core.CommandID]*host.CommandRecord
	running                  map[core.CommandID]*runningCommand
	defaultShell, spareShell *core.ShellID
}

// NewShellEnvironment constructs a node's environment.
func NewShellEnvironment(options EnvironmentOptions) *ShellEnvironment {
	return &ShellEnvironment{
		options: options,
		shells:  make(map[core.ShellID]*shellState),
		records: make(map[core.CommandID]*host.CommandRecord),
		running: make(map[core.CommandID]*runningCommand),
	}
}

// Exec starts a shell command and observes it for the requested window.
func (e *ShellEnvironment) Exec(ctx context.Context, request host.ExecRequest) (host.CommandStatus, error) {
	command, running, err := e.start(ctx, request)
	if err != nil {
		return host.CommandStatus{}, err
	}
	timer := time.NewTimer(request.Window.Duration())
	defer timer.Stop()
	select {
	case <-running.settled:
	case <-timer.C:
	case <-ctx.Done():
		return host.CommandStatus{}, ctx.Err()
	}
	return e.Status(command)
}

// Status returns the command's status and output since the previous look.
func (e *ShellEnvironment) Status(id core.CommandID) (host.CommandStatus, error) {
	e.mu.Lock()
	record := e.records[id]
	running := e.running[id]
	e.mu.Unlock()
	if record == nil {
		return host.CommandStatus{}, &host.ShellError{Kind: host.UnknownCommand, Command: id}
	}
	var hint *string
	if running != nil && record.IsRunning() {
		running.mu.Lock()
		job := running.job
		running.mu.Unlock()
		if job != nil {
			hint = job.RunningHint()
		}
	}
	return record.Status(e.options.OutputLimit, hint), nil
}

// ReadOutput reads a command's whole kept output.
func (e *ShellEnvironment) ReadOutput(ctx context.Context, id core.CommandID) (host.WholeOutput, error) {
	running, err := e.active(id)
	if err != nil {
		return host.WholeOutput{}, err
	}
	job, err := running.started(ctx)
	if err != nil {
		return host.WholeOutput{}, err
	}
	if job == nil {
		return host.WholeOutput{}, &host.ShellError{Kind: host.NotRunning, Command: id}
	}
	return job.ReadOutput(ctx)
}

// Write supplies live standard input to a running command.
func (e *ShellEnvironment) Write(ctx context.Context, id core.CommandID, bytes []byte) error {
	running, err := e.active(id)
	if err != nil {
		return err
	}
	if len(bytes) == 0 {
		return &host.ShellError{Kind: host.EmptyStdin}
	}
	job, err := running.started(ctx)
	if err != nil {
		return err
	}
	if job == nil {
		return &host.ShellError{Kind: host.NotRunning, Command: id}
	}
	return job.WriteStdin(ctx, bytes)
}

// Abort terminates a command and joins its output and edit publication.
func (e *ShellEnvironment) Abort(ctx context.Context, id core.CommandID) error {
	e.mu.Lock()
	record := e.records[id]
	running := e.running[id]
	e.mu.Unlock()
	if record == nil {
		return &host.ShellError{Kind: host.UnknownCommand, Command: id}
	}
	if running == nil || !record.IsRunning() {
		return nil
	}
	running.mu.Lock()
	running.aborted = true
	running.mu.Unlock()
	running.cancel()
	settled, err := running.within(ctx, 5*time.Second)
	if err != nil {
		return err
	}
	if settled {
		return nil
	}
	running.mu.Lock()
	job := running.job
	running.mu.Unlock()
	if job != nil {
		if err := job.Kill(ctx, runnerwire.Signal("SIGKILL")); err != nil {
			slog.Debug("could not kill the job: "+err.Error(), "command", id)
		}
	}
	settled, err = running.within(ctx, 5*time.Second)
	if err != nil {
		return err
	}
	if !settled && record.MarkAborted() {
		e.options.Feed.Changed(record)
	}
	return nil
}

// PageViews returns the commands' current page views.
func (e *ShellEnvironment) PageViews() []host.PageView {
	e.mu.Lock()
	records := make([]*host.CommandRecord, 0, len(e.records))
	for _, record := range e.records {
		records = append(records, record)
	}
	e.mu.Unlock()
	views := make([]host.PageView, 0, len(records))
	for _, record := range records {
		views = append(views, record.PageView())
	}
	return views
}

// ReleaseCommand disposes a command handle and reports whether it existed.
func (e *ShellEnvironment) ReleaseCommand(ctx context.Context, id core.CommandID) bool {
	e.mu.Lock()
	record := e.records[id]
	e.mu.Unlock()
	if record == nil {
		return false
	}
	if record.IsRunning() {
		if err := e.Abort(ctx, id); err != nil {
			slog.Debug("could not abort the released command: "+err.Error(), "command", id)
		}
	}
	e.mu.Lock()
	delete(e.records, id)
	e.mu.Unlock()
	return true
}

// DisposeShell disposes a shell and its commands, reporting whether it existed.
func (e *ShellEnvironment) DisposeShell(ctx context.Context, id core.ShellID) bool {
	e.mu.Lock()
	shell := e.shells[id]
	var foreground *core.CommandID
	if shell != nil && shell.foreground != nil {
		foreground = new(*shell.foreground)
	}
	e.mu.Unlock()
	if shell == nil {
		return false
	}
	if foreground != nil {
		if err := e.Abort(ctx, *foreground); err != nil {
			slog.Debug("could not abort the shell's command: "+err.Error(), "command", *foreground)
		}
	}
	e.mu.Lock()
	delete(e.shells, id)
	if e.defaultShell != nil && *e.defaultShell == id {
		e.defaultShell = nil
	}
	e.mu.Unlock()
	return true
}

// DisposeAll cancels and joins every owned job and releases every shell.
func (e *ShellEnvironment) DisposeAll(ctx context.Context) error {
	e.mu.Lock()
	shells := make([]core.ShellID, 0, len(e.shells))
	for shell := range e.shells {
		shells = append(shells, shell)
	}
	e.mu.Unlock()
	for _, shell := range shells {
		e.DisposeShell(ctx, shell)
	}
	e.mu.Lock()
	running := make([]*runningCommand, 0, len(e.running))
	for _, command := range e.running {
		running = append(running, command)
	}
	e.mu.Unlock()
	for _, command := range running {
		<-command.settled
	}
	e.tasks.Wait()
	return ctx.Err()
}

// OwnsShell reports whether the shell belongs to this environment.
func (e *ShellEnvironment) OwnsShell(id core.ShellID) bool {
	e.mu.Lock()
	defer e.mu.Unlock()
	_, ok := e.shells[id]
	return ok
}

// OwnsCommand reports whether the command belongs to this environment.
func (e *ShellEnvironment) OwnsCommand(id core.CommandID) bool {
	e.mu.Lock()
	defer e.mu.Unlock()
	_, ok := e.records[id]
	return ok
}

// EditedFile builds a file's page record with stored copies for each edit segment.
func EditedFile(file runnerwire.JobFileChange, copies func(int) *core.EditCopies) core.EditedFile {
	edited := core.EditedFile{
		Path:    file.Path,
		Kind:    core.EditKind(file.Kind),
		Added:   uint32(min(file.Added, math.MaxUint32)),
		Removed: uint32(min(file.Removed, math.MaxUint32)),
		Edits:   make([]core.EditSegment, len(file.Edits)),
	}
	for i := range edited.Edits {
		edited.Edits[i].Copies = copies(i)
	}
	return edited
}

var _ host.ShellEnvironment = (*ShellEnvironment)(nil)
