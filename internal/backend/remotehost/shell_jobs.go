package remotehost

import (
	"context"
	"errors"
	"log/slog"
	"maps"
	"strconv"
	"sync"
	"time"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// errShellAborted preserves the shell cancellation text shown to callers.
//
//nolint:staticcheck // ST1005: product text, shown to the user as it is.
var errShellAborted = errors.New("Shell command aborted")

type shellState struct {
	cwd        string
	env        map[string]string
	foreground core.CommandID // Empty while the shell is idle.
}

// runningCommand owns the task between Host admission and final output publication.
type runningCommand struct {
	ctx        context.Context
	cancel     context.CancelFunc
	mu         sync.Mutex // Protects job publication and the explicit abort marker.
	job        *Job
	startedJob chan struct{}
	settled    chan struct{}
	aborted    bool
	received   []host.OutputRecord // Owned only by the command task.
}

// started waits for admission to produce a job or for the command to settle without one.
func (r *runningCommand) started(ctx context.Context) (*Job, error) {
	select {
	case <-r.startedJob:
	case <-r.settled:
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.job, nil
}

// within observes command settlement for the shell tool's specified window.
func (r *runningCommand) within(ctx context.Context, duration time.Duration) (bool, error) {
	timer := time.NewTimer(duration)
	defer timer.Stop()
	select {
	case <-r.settled:
		return true, nil
	case <-timer.C:
		return false, nil
	case <-ctx.Done():
		return false, ctx.Err()
	}
}

// active rejects unknown or finished commands before their IO is attempted.
func (e *ShellEnvironment) active(id core.CommandID) (*runningCommand, error) {
	e.mu.Lock()
	record := e.records[id]
	running := e.running[id]
	e.mu.Unlock()
	if record == nil {
		return nil, &host.ShellError{Kind: host.UnknownCommand, Command: id}
	}
	if running == nil || !record.IsRunning() {
		return nil, &host.ShellError{Kind: host.NotRunning, Command: id}
	}
	return running, nil
}

// checkFreeLocked checks a shell reservation under the environment mutex.
func (e *ShellEnvironment) checkFreeLocked(id core.ShellID) error {
	shell := e.shells[id]
	if shell == nil {
		return &host.ShellError{Kind: host.UnknownShell, Shell: id}
	}
	if shell.foreground != "" {
		return &host.ShellError{Kind: host.ShellBusy, Shell: id, Command: shell.foreground}
	}
	return nil
}

// reserve atomically selects an idle shell or requests a new shell number.
func (e *ShellEnvironment) reserve(target host.ShellTarget, command core.CommandID) (core.ShellID, bool, error) {
	e.mu.Lock()
	defer e.mu.Unlock()
	var id core.ShellID
	if target.Kind == host.ExistingShell {
		if err := e.checkFreeLocked(target.ID); err != nil {
			return "", false, err
		}
		id = target.ID
	} else if target.Kind == host.DefaultShell && e.defaultShell != "" && e.shells[e.defaultShell].foreground == "" {
		id = e.defaultShell
	} else {
		var ok bool
		id, ok = e.reserveNewShellLocked(target)
		if !ok {
			return "", false, nil
		}
	}
	e.shells[id].foreground = command
	return id, true, nil
}

// start takes sequence numbers outside locks and registers one owned shell task.
func (e *ShellEnvironment) start(
	ctx context.Context,
	request host.ExecRequest,
) (core.CommandID, *runningCommand, error) {
	if request.Shell.Kind == host.ExistingShell {
		e.mu.Lock()
		err := e.checkFreeLocked(request.Shell.ID)
		e.mu.Unlock()
		if err != nil {
			return "", nil, err
		}
	}
	number, err := e.options.Numbers.Next(ctx, core.SequenceCommand)
	if err != nil {
		return "", nil, err
	}
	command := core.CommandID(strconv.FormatUint(number, 10))
	var shell core.ShellID
	for {
		selected, ok, err := e.reserve(request.Shell, command)
		if err != nil {
			return "", nil, err
		}
		if ok {
			shell = selected
			break
		}
		number, err := e.options.Numbers.Next(ctx, core.SequenceShell)
		if err != nil {
			return "", nil, err
		}
		spare := core.ShellID(strconv.FormatUint(number, 10))
		e.mu.Lock()
		e.spareShell = spare
		e.mu.Unlock()
	}
	record := host.NewCommandRecord(shell, command, request.ToolUseID)
	lifetime, cancel := context.WithCancel(ctx)
	running := &runningCommand{
		ctx:        lifetime,
		cancel:     cancel,
		startedJob: make(chan struct{}),
		settled:    make(chan struct{}),
	}
	e.mu.Lock()
	e.records[command] = record
	e.running[command] = running
	e.tasks.Add(1)
	e.mu.Unlock()
	e.options.Feed.Changed(record)
	go e.run(shell, command, request, running, record)
	return command, running, nil
}

// run keeps Host admission through edits, output storage and the runner's job release.
func (e *ShellEnvironment) run(
	shell core.ShellID,
	command core.CommandID,
	request host.ExecRequest,
	running *runningCommand,
	record *host.CommandRecord,
) {
	defer e.tasks.Done()
	var failure error
	job := func(ctx context.Context) error {
		failure = e.execute(ctx, shell, command, request, running, record)
		return nil
	}
	if e.options.Access != nil {
		if err := e.options.Access.RunJob(running.ctx, e.options.Host.Key(), job); err != nil {
			failure = err
		}
	} else {
		failure = e.execute(running.ctx, shell, command, request, running, record)
	}
	if failure != nil && record.IsRunning() {
		ending := host.Ending{Phase: host.Exited, ExitCode: 127}
		page := ""
		if running.ctx.Err() != nil {
			ending.Phase = host.Aborted
		} else {
			page = pushReason(&running.received, failure.Error())
		}
		e.settle(
			context.WithoutCancel(running.ctx),
			command,
			record,
			ending,
			host.WholeOutput{Records: running.received},
			nil,
			page,
			nil,
		)
	}
	e.mu.Lock()
	if state := e.shells[shell]; state != nil && state.foreground == command {
		state.foreground = ""
	}
	running.cancel()
	close(running.settled)
	delete(e.running, command)
	e.mu.Unlock()
}

// execute follows one runner job while preserving the command's cancellation and page demand.
func (e *ShellEnvironment) execute(
	ctx context.Context,
	shell core.ShellID,
	command core.CommandID,
	request host.ExecRequest,
	running *runningCommand,
	record *host.CommandRecord,
) error {
	commandContext, err := e.options.Context(ctx)
	if err != nil {
		return err
	}
	if running.ctx.Err() != nil {
		return errShellAborted
	}
	e.mu.Lock()
	state := e.shells[shell]
	if state == nil {
		e.mu.Unlock()
		return errShellAborted
	}
	cwd := state.cwd
	env := maps.Clone(state.env)
	e.mu.Unlock()
	if env == nil {
		env = make(map[string]string)
	}
	env["PWD"] = cwd
	job, err := e.options.Host.StartJob(
		ctx,
		JobStart{
			Script:   request.Script,
			CWD:      cwd,
			Env:      env,
			Context:  commandContext,
			Caller:   new(request.Caller),
			Commands: e.options.Commands,
		},
	)
	if err != nil {
		return err
	}
	running.mu.Lock()
	running.job = job
	running.mu.Unlock()
	close(running.startedJob)
	streams := e.followJob(ctx, command, running, record, job)
	workCtx := context.WithoutCancel(ctx)
	end, err := job.End(workCtx)
	if err != nil {
		return err
	}
	e.finishJob(workCtx, shell, command, running, record, job, end, streams)
	return nil
}

// reserveNewShellLocked consumes a spare number and initializes a shell under the environment mutex.
func (e *ShellEnvironment) reserveNewShellLocked(target host.ShellTarget) (core.ShellID, bool) {
	if e.spareShell == "" {
		return "", false
	}
	id := e.spareShell
	e.spareShell = ""
	cwd := e.options.Host.DefaultCWD()
	if target.Kind == host.EphemeralShell && target.CWD != nil {
		cwd = *target.CWD
	}
	e.shells[id] = &shellState{cwd: cwd, env: maps.Clone(e.options.InitialEnv)}
	if target.Kind == host.DefaultShell && e.defaultShell == "" {
		e.defaultShell = id
	}
	return id, true
}

// followJob receives job output and forwards following and interruption decisions until exit.
func (e *ShellEnvironment) followJob(
	ctx context.Context,
	command core.CommandID,
	running *runningCommand,
	record *host.CommandRecord,
	job *Job,
) [2]receivedStream {
	var streams [2]receivedStream
	followed := false
	stop := running.ctx.Done()
	workCtx := context.WithoutCancel(ctx)
	for {
		follow, watch := e.options.Feed.Watching()
		if follow != followed {
			followed = follow
			if err := job.Follow(workCtx, follow); err != nil {
				slog.Debug("could not change the job's following: "+err.Error(), "command", command)
			}
		}
		chunk, available, ended, changed := job.state.poll()
		if available {
			index := 0
			if chunk.Stream == core.StreamKind("stderr") {
				index = 1
			}
			if streams[index].receive(record, chunk, &running.received) {
				e.options.Feed.Changed(record)
			}
			continue
		}
		if ended {
			break
		}
		select {
		case <-changed:
		case <-watch:
		case <-stop:
			stop = nil
			if err := job.Kill(workCtx, runnerwire.Signal("SIGTERM")); err != nil {
				slog.Debug("could not interrupt the job: "+err.Error(), "command", command)
			}
		}
	}
	return streams
}
