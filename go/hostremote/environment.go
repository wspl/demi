package hostremote

import (
	"cmp"
	"context"
	"errors"
	"maps"
	"math"
	"slices"
	"strconv"
	"sync"
	"time"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
)

const AbortGrace = 5 * time.Second

type HostAccess interface {
	RunJob(context.Context, shell.HostKey, func(context.Context) error) error
}
type CommandKeeper interface {
	Retain(context.Context, core.CommandID, []runnerproto.JobFileChange) []core.EditedFile
	KeepOutput(context.Context, core.CommandID, *shell.WholeOutput)
}
type ContextSource func(context.Context) (commandservice.CommandContext, error)
type EnvironmentOptions struct {
	Host                     *RemoteHost
	Execute                  Executor
	Commands                 *CommandSelection
	Context                  ContextSource
	Feed                     shell.PageFeed
	Numbers                  shell.Numbers
	Access                   HostAccess
	Keeper                   CommandKeeper
	InitialEnv               map[string]string
	OutputLimit, BinaryLimit int
}

func NewEnvironmentOptions(host *RemoteHost, execute Executor, source ContextSource, feed shell.PageFeed, numbers shell.Numbers) EnvironmentOptions {
	return EnvironmentOptions{Host: host, Execute: execute, Context: source, Feed: feed, Numbers: numbers, OutputLimit: shell.DefaultOutputLimitBytes, BinaryLimit: shell.DefaultBinaryLimitBytes}
}

type RemoteShellEnvironmentFactory struct{ Catalog *CommandCatalog }

func (f RemoteShellEnvironmentFactory) Create(commands *shell.CommandSet, options EnvironmentOptions) (*RemoteShellEnvironment, error) {
	selection, err := f.Catalog.Select(commands)
	if err != nil {
		return nil, err
	}
	options.Commands = selection
	return NewRemoteShellEnvironment(options), nil
}

// Environment state is touched only in Execute closures. Each running command
// owns its waiting goroutine; disposal cancels and joins those goroutines.
type RemoteShellEnvironment struct {
	options                  EnvironmentOptions
	shells                   map[core.ShellID]*remoteShell
	defaultShell, spareShell *core.ShellID
	records                  map[core.CommandID]*shell.CommandRecord
	running                  map[core.CommandID]*runningCommand
	disposing                chan struct{}
	tasks                    sync.WaitGroup
}
type remoteShell struct {
	cwd        string
	env        map[string]string
	foreground *core.CommandID
}
type runningCommand struct {
	ctx     context.Context
	cancel  context.CancelFunc
	job     *RemoteJob // shard-owned
	aborted bool       // shard-owned explicit abort, distinct from invocation cancellation
	settled chan struct{}
}

func NewRemoteShellEnvironment(options EnvironmentOptions) *RemoteShellEnvironment {
	return &RemoteShellEnvironment{options: options, shells: map[core.ShellID]*remoteShell{}, records: map[core.CommandID]*shell.CommandRecord{}, running: map[core.CommandID]*runningCommand{}}
}
func (e *RemoteShellEnvironment) checkFree(id core.ShellID) error {
	s := e.shells[id]
	if s == nil {
		return &shell.ShellError{Kind: shell.UnknownShell, Shell: id}
	}
	if s.foreground != nil {
		return &shell.ShellError{Kind: shell.ShellBusy, Shell: id, Command: *s.foreground}
	}
	return nil
}
func (e *RemoteShellEnvironment) reserve(target shell.ShellTarget, command core.CommandID) (*core.ShellID, error) {
	if e.disposing != nil {
		return nil, nil
	}
	var id *core.ShellID
	switch target.Kind {
	case shell.ShellExisting:
		if err := e.checkFree(target.ID); err != nil {
			return nil, err
		}
		id = &target.ID
	case shell.ShellDefault:
		if e.defaultShell != nil && e.shells[*e.defaultShell].foreground == nil {
			id = e.defaultShell
		}
	}
	if id == nil {
		if e.spareShell == nil {
			return nil, nil
		}
		id = e.spareShell
		e.spareShell = nil
		cwd := e.options.Host.DefaultCwd()
		if target.Kind == shell.ShellEphemeral && target.Cwd != nil {
			cwd = *target.Cwd
		}
		e.shells[*id] = &remoteShell{cwd: cwd, env: maps.Clone(e.options.InitialEnv)}
		if target.Kind == shell.ShellDefault && e.defaultShell == nil {
			e.defaultShell = id
		}
	}
	e.shells[*id].foreground = &command
	return id, nil
}
func (e *RemoteShellEnvironment) Exec(ctx context.Context, request shell.ExecRequest) (shell.CommandStatus, error) {
	var failure error
	if err := e.awaitReady(ctx); err != nil {
		return shell.CommandStatus{}, err
	}
	if err := e.options.Execute(ctx, func() {
		if request.Shell.Kind == shell.ShellExisting {
			failure = e.checkFree(request.Shell.ID)
		}
	}); err != nil {
		return shell.CommandStatus{}, err
	}
	if failure != nil {
		return shell.CommandStatus{}, failure
	}
	number, err := e.options.Numbers.Next(ctx, core.SequenceCommand)
	if err != nil {
		return shell.CommandStatus{}, err
	}
	command, err := core.ParseCommandID(strconv.FormatUint(number, 10))
	if err != nil {
		return shell.CommandStatus{}, err
	}
	lifetime, cancel := context.WithCancel(ctx)
	running := &runningCommand{ctx: lifetime, cancel: cancel, settled: make(chan struct{})}
	var id *core.ShellID
	for id == nil {
		if err := e.awaitReady(ctx); err != nil {
			cancel()
			return shell.CommandStatus{}, err
		}
		err = e.options.Execute(ctx, func() {
			id, failure = e.reserve(request.Shell, command)
			if id == nil || failure != nil {
				return
			}
			record := shell.NewCommandRecord(*id, command, request.ToolUseID)
			e.records[command] = record
			e.running[command] = running
			e.options.Feed.Changed(record.PageView())
			e.tasks.Add(1)
			go func() {
				defer e.tasks.Done()
				e.run(*id, command, request.Script, request.Caller, running, record)
			}()
		})
		if err != nil || failure != nil {
			cancel()
			if err != nil {
				return shell.CommandStatus{}, err
			}
			return shell.CommandStatus{}, failure
		}
		if id == nil {
			number, err := e.options.Numbers.Next(ctx, core.SequenceShell)
			if err != nil {
				cancel()
				return shell.CommandStatus{}, err
			}
			spare, err := core.ParseShellID(strconv.FormatUint(number, 10))
			if err != nil {
				cancel()
				return shell.CommandStatus{}, err
			}
			if err := e.options.Execute(ctx, func() { e.spareShell = &spare }); err != nil {
				cancel()
				return shell.CommandStatus{}, err
			}
		}
	}
	timer := time.NewTimer(request.Window.Duration())
	defer timer.Stop()
	select {
	case <-running.settled:
	case <-timer.C:
	case <-ctx.Done():
	}
	return e.Status(command)
}
func (e *RemoteShellEnvironment) Status(command core.CommandID) (shell.CommandStatus, error) {
	var status shell.CommandStatus
	var failure error
	err := e.options.Execute(context.Background(), func() {
		record := e.records[command]
		if record == nil {
			failure = &shell.ShellError{Kind: shell.UnknownCommand, Command: command}
			return
		}
		var hint *string
		if r := e.running[command]; r != nil && r.job != nil {
			hint = r.job.RunningHint()
		}
		status = record.Status(e.options.OutputLimit, hint)
	})
	if err != nil {
		return status, err
	}
	return status, failure
}
func (e *RemoteShellEnvironment) PageViews() []shell.PageView {
	var views []shell.PageView
	// A retired shard has no views left to publish.
	_ = e.options.Execute(context.Background(), func() {
		for _, r := range e.records {
			views = append(views, r.PageView())
		}
	})
	slices.SortFunc(views, func(a, b shell.PageView) int { return cmp.Compare(a.CommandID.String(), b.CommandID.String()) })
	return views
}
func (e *RemoteShellEnvironment) OwnsShell(id core.ShellID) bool {
	var owns bool
	_ = e.options.Execute(context.Background(), func() { _, owns = e.shells[id] })
	return owns
}
func (e *RemoteShellEnvironment) OwnsCommand(id core.CommandID) bool {
	var owns bool
	_ = e.options.Execute(context.Background(), func() { _, owns = e.records[id] })
	return owns
}
func (e *RemoteShellEnvironment) active(ctx context.Context, id core.CommandID, empty bool) (*RemoteJob, error) {
	var job *RemoteJob
	var failure error
	err := e.options.Execute(ctx, func() {
		record := e.records[id]
		switch {
		case record == nil:
			failure = &shell.ShellError{Kind: shell.UnknownCommand, Command: id}
		case !record.IsRunning():
			failure = &shell.ShellError{Kind: shell.NotRunning, Command: id}
		case empty:
			failure = &shell.ShellError{Kind: shell.EmptyStdin}
		default:
			if r := e.running[id]; r != nil {
				job = r.job
			}
			if job == nil {
				failure = &shell.ShellError{Kind: shell.Starting, Command: id}
			}
		}
	})
	if err != nil {
		return nil, err
	}
	return job, failure
}
func (e *RemoteShellEnvironment) ReadOutput(ctx context.Context, id core.CommandID) (*shell.WholeOutput, error) {
	job, err := e.active(ctx, id, false)
	if err != nil {
		return nil, err
	}
	return job.ReadOutput(ctx)
}
func (e *RemoteShellEnvironment) Write(ctx context.Context, id core.CommandID, bytes []byte) error {
	job, err := e.active(ctx, id, len(bytes) == 0)
	if err != nil {
		return err
	}
	return job.WriteStdin(ctx, bytes)
}
func (e *RemoteShellEnvironment) Abort(ctx context.Context, id core.CommandID) error {
	var running *runningCommand
	var failure error
	err := e.options.Execute(ctx, func() {
		record := e.records[id]
		if record == nil {
			failure = &shell.ShellError{Kind: shell.UnknownCommand, Command: id}
			return
		}
		if record.IsRunning() {
			running = e.running[id]
		}
		if running != nil {
			running.aborted = true
			running.cancel()
		}
	})
	if err != nil {
		return err
	}
	if failure != nil || running == nil {
		return failure
	}
	if settledWithin(ctx, running.settled, AbortGrace) {
		return nil
	}
	var job *RemoteJob
	if err := e.options.Execute(ctx, func() { job = running.job }); err != nil {
		return err
	}
	if job != nil {
		// A disconnected runner's shared end is the authoritative failure.
		_ = job.Kill(ctx, runnerproto.SignalKill)
	}
	if !settledWithin(ctx, running.settled, AbortGrace) {
		return e.options.Execute(ctx, func() {
			if record := e.records[id]; record != nil && record.MarkAborted() {
				e.options.Feed.Changed(record.PageView())
			}
		})
	}
	return nil
}
func settledWithin(ctx context.Context, settled <-chan struct{}, within time.Duration) bool {
	timer := time.NewTimer(within)
	defer timer.Stop()
	select {
	case <-settled:
		return true
	case <-ctx.Done():
		return false
	case <-timer.C:
		return false
	}
}
func (e *RemoteShellEnvironment) ReleaseCommand(ctx context.Context, id core.CommandID) bool {
	if !e.OwnsCommand(id) {
		return false
	}
	if err := e.Abort(ctx, id); err != nil {
		return false
	}
	return e.options.Execute(ctx, func() { delete(e.records, id) }) == nil
}
func (e *RemoteShellEnvironment) DisposeShell(ctx context.Context, id core.ShellID) bool {
	var exists bool
	var foreground *core.CommandID
	if err := e.options.Execute(ctx, func() {
		if s := e.shells[id]; s != nil {
			exists = true
			foreground = s.foreground
		}
	}); err != nil || !exists {
		return false
	}
	if foreground != nil {
		if err := e.Abort(ctx, *foreground); err != nil {
			return false
		}
	}
	return e.options.Execute(ctx, func() {
		delete(e.shells, id)
		if e.defaultShell != nil && *e.defaultShell == id {
			e.defaultShell = nil
		}
	}) == nil
}

// awaitReady keeps execution off the shard while an environment disposes its
// current shells. Disposal is reusable, as in the Rust environment contract.
func (e *RemoteShellEnvironment) awaitReady(ctx context.Context) error {
	for {
		var disposing <-chan struct{}
		if err := e.options.Execute(ctx, func() { disposing = e.disposing }); err != nil {
			return err
		}
		if disposing == nil {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-disposing:
		}
	}
}
func (e *RemoteShellEnvironment) DisposeAll(ctx context.Context) error {
	var ids []core.ShellID
	var previous <-chan struct{}
	// Teardown owns cancellation even when its caller has already cancelled.
	cleanup := context.WithoutCancel(ctx)
	if err := e.options.Execute(cleanup, func() {
		previous = e.disposing
		if previous != nil {
			return
		}
		e.disposing = make(chan struct{})
		for id := range e.shells {
			ids = append(ids, id)
		}
	}); err != nil {
		return err
	}
	if previous != nil {
		<-previous
		return ctx.Err()
	}
	for _, id := range ids {
		e.DisposeShell(cleanup, id)
	}
	e.tasks.Wait()
	if err := e.options.Execute(cleanup, func() {
		close(e.disposing)
		e.disposing = nil
	}); err != nil {
		return err
	}
	return ctx.Err()
}

func (e *RemoteShellEnvironment) run(id core.ShellID, command core.CommandID, script string, caller shell.JobCaller, running *runningCommand, record *shell.CommandRecord) {
	defer close(running.settled)
	defer running.cancel()
	var received []shell.OutputRecord
	execute := func(ctx context.Context) error {
		return e.execute(ctx, id, command, script, caller, running, record, &received)
	}
	var failure error
	if e.options.Access != nil {
		failure = e.options.Access.RunJob(running.ctx, e.options.Host.Key(), execute)
	} else {
		failure = execute(running.ctx)
	}
	if failure != nil {
		ending := shell.Ending{Aborted: running.ctx.Err() != nil, ExitCode: 127}
		page := ""
		if !ending.Aborted {
			page = pushReason(&received, failure.Error())
		}
		e.end(command, record, ending, shell.NewWholeOutput(received, nil), nil, page, nil)
	}
	// A retired shard has disposed its records; worker cleanup still joins.
	_ = e.options.Execute(context.Background(), func() {
		delete(e.running, command)
		if s := e.shells[id]; s != nil && s.foreground != nil && *s.foreground == command {
			s.foreground = nil
		}
	})
}
func (e *RemoteShellEnvironment) execute(ctx context.Context, id core.ShellID, command core.CommandID, script string, caller shell.JobCaller, running *runningCommand, record *shell.CommandRecord, received *[]shell.OutputRecord) error {
	identity, err := e.options.Context(ctx)
	if err != nil {
		return err
	}
	if ctx.Err() != nil {
		return errors.New("Shell command aborted")
	}
	var cwd string
	var env map[string]string
	if err := e.options.Execute(ctx, func() {
		if s := e.shells[id]; s != nil {
			cwd = s.cwd
			env = maps.Clone(s.env)
		}
	}); err != nil {
		return err
	}
	if env == nil {
		env = map[string]string{}
	}
	env["PWD"] = cwd
	job, err := e.options.Host.StartJob(ctx, JobStart{Script: script, Cwd: cwd, Env: env, Context: identity, Caller: &caller, Commands: e.options.Commands})
	if err != nil {
		return err
	}
	if err := e.options.Execute(context.Background(), func() { running.job = job }); err != nil {
		job.Kill(context.Background(), runnerproto.SignalKill)
		return err
	}
	streams := [2]receivedStream{}
	followed, signalled := false, false
	for {
		watching, changed := e.options.Feed.Watching()
		if watching != followed {
			followed = watching
			_ = job.Follow(context.Background(), followed)
		}
		job.shared.mu.Lock()
		var chunk *JobOutput
		if len(job.shared.output) > 0 {
			value := job.shared.output[0]
			job.shared.output[0] = JobOutput{}
			job.shared.output = job.shared.output[1:]
			chunk = &value
		}
		ended, outputChanged := job.shared.end != nil, job.shared.changed
		job.shared.mu.Unlock()
		if chunk != nil {
			if err := e.options.Execute(context.Background(), func() {
				index := 0
				if chunk.Stream == core.StreamKindStderr {
					index = 1
				}
				if streams[index].receive(record, *chunk, received) {
					e.options.Feed.Changed(record.PageView())
				}
			}); err != nil {
				job.Kill(context.Background(), runnerproto.SignalKill)
				return err
			}
			continue
		}
		if ended {
			break
		}
		var stopped <-chan struct{}
		if !signalled {
			stopped = ctx.Done()
		}
		select {
		case <-outputChanged:
		case <-changed:
		case <-stopped:
			signalled = true
			_ = job.Kill(context.Background(), runnerproto.SignalTerminate)
		}
	}
	end, err := job.End(context.Background())
	if err != nil {
		return err
	}
	e.finish(id, command, running, record, job, end, streams, *received)
	return nil
}
func (e *RemoteShellEnvironment) end(command core.CommandID, record *shell.CommandRecord, ending shell.Ending, output *shell.WholeOutput, binary *shell.BinaryOutput, page string, job *RemoteJob) {
	if e.options.Keeper != nil {
		e.options.Keeper.KeepOutput(context.Background(), command, output)
	}
	if job != nil {
		_ = job.Release(context.Background())
	}
	_ = e.options.Execute(context.Background(), func() {
		if record.Settle(ending, output, binary, page) {
			e.options.Feed.Changed(record.PageView())
		}
	})
}
func (e *RemoteShellEnvironment) finish(id core.ShellID, command core.CommandID, running *runningCommand, record *shell.CommandRecord, job *RemoteJob, end JobEnd, streams [2]receivedStream, received []shell.OutputRecord) {
	if len(end.Files) > 0 {
		var files []core.EditedFile
		if e.options.Keeper != nil {
			files = e.options.Keeper.Retain(context.Background(), command, end.Files)
		} else {
			for _, file := range end.Files {
				files = append(files, EditedFile(file, func(int) *core.EditCopies { return nil }))
			}
		}
		_ = e.options.Execute(context.Background(), func() { record.SetFiles(shell.EditedFiles{Files: files, Truncated: end.FilesTruncated}) })
	}
	if end.Cwd != nil {
		_ = e.options.Execute(context.Background(), func() {
			if s := e.shells[id]; s != nil {
				s.cwd = *end.Cwd
			}
		})
	}
	exitCode := int32(127)
	switch end.Status.Kind {
	case shell.ProcessNotStarted:
		reason := "bash: " + string(end.Status.Error.Kind)
		if end.Status.Error.Kind == shell.SpawnOther && end.Status.Error.Detail != nil {
			reason = *end.Status.Error.Detail
		}
		page := pushReason(&received, reason)
		e.end(command, record, shell.Ending{ExitCode: 127}, shell.NewWholeOutput(received, nil), nil, page, job)
		return
	case shell.ProcessLost:
		var missing *shell.Missing
		n := streams[0].unreceived() + streams[1].unreceived()
		if n > 0 {
			missing = &shell.Missing{Bytes: n, Reason: "lost with the Host's connection"}
		}
		page := pushReason(&received, end.Status.Reason)
		if missing != nil {
			page += missing.Line() + "\n"
		}
		e.end(command, record, shell.Ending{ExitCode: 127}, shell.NewWholeOutput(received, missing), nil, page, job)
		return
	case shell.ProcessExited:
		exitCode = end.Status.ExitCode
	case shell.ProcessSignalled:
		exitCode = 128
		if end.Status.Signal == "SIGTERM" || end.Status.Signal == "SIGKILL" {
			exitCode = 130
		}
	}
	lengths := runnerproto.OutputLengths{StdoutBytes: streams[0].head, StderrBytes: streams[1].head}
	if end.Output != nil {
		lengths = *end.Output
	}
	unreceived := saturatingSubtract(lengths.StdoutBytes, streams[0].head) + saturatingSubtract(lengths.StderrBytes, streams[1].head)
	output := shell.NewWholeOutput(received, nil)
	page := ""
	read := false
	if unreceived > 0 {
		kept, err := job.ReadOutput(context.Background())
		if err == nil {
			output = kept
			read = true
		} else {
			missing := &shell.Missing{Bytes: unreceived, Reason: "not read from the Host: " + err.Error()}
			output = shell.NewWholeOutput(received, missing)
			page = missing.Line() + "\n"
		}
	}
	binary := output.BinaryStdout(lengths.StdoutBytes, e.options.BinaryLimit)
	var binaryLength *uint64
	if binary != nil {
		binaryLength = &binary.Info.TotalBytes
	}
	if read {
		page = output.Text(shell.Streams{}, binaryLength, shell.Seen{}).Display()
	} else if binaryLength != nil {
		page = shell.BinaryLine(*binaryLength) + "\n" + page
	}
	ending := shell.Ending{ExitCode: exitCode}
	_ = e.options.Execute(context.Background(), func() {
		ending.Aborted = running.aborted
		record.Grew(core.StreamKindStdout, lengths.StdoutBytes)
		record.Grew(core.StreamKindStderr, lengths.StderrBytes)
	})
	e.end(command, record, ending, output, binary, page, job)
}
func EditedFile(file runnerproto.JobFileChange, copies func(int) *core.EditCopies) core.EditedFile {
	edits := make([]core.EditSegment, len(file.Edits))
	for i := range edits {
		edits[i] = core.EditSegment{Copies: copies(i)}
	}
	return core.EditedFile{Path: file.Path, Kind: core.EditKind(file.Kind), Added: uint32(min(file.Added, math.MaxUint32)), Removed: uint32(min(file.Removed, math.MaxUint32)), Edits: edits}
}
func pushReason(output *[]shell.OutputRecord, reason string) string {
	separator := ""
	for i := len(*output) - 1; i >= 0; i-- {
		if read, ok := (*output)[i].(shell.OutputRead); ok && len(read.Bytes) > 0 {
			if read.Bytes[len(read.Bytes)-1] != '\n' {
				separator = "\n"
			}
			break
		}
	}
	text := separator + reason + "\n"
	*output = append(*output, shell.OutputRead{Stream: core.StreamKindStderr, Bytes: []byte(text)})
	return text
}
func saturatingSubtract(a, b uint64) uint64 {
	if a > b {
		return a - b
	}
	return 0
}

var _ shell.ShellEnvironment = (*RemoteShellEnvironment)(nil)
