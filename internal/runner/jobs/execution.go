package jobs

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"path/filepath"
	"sync"
	"time"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// taskExecution keeps stream handles apart from the process or shell owner.
type taskExecution struct {
	input  chan<- process.Input
	output <-chan process.OutputChunk
	cancel func()
	signal func(context.Context, runnerwire.Signal) error
	wait   func(context.Context) (process.Exit, *string)
}

// run owns a task's shell/process, retained data and pipe workers through completion.
func (t *Table) run(spec TaskSpec, entry *taskEntry) (frame []byte, err error) {
	ctx := entry.lifetime
	var child taskExecution
	switch command := spec.Command.(type) {
	case *ProcessCommand:
		started, err := process.Spawn(
			ctx,
			process.SpawnOptions{
				Command:      command.Command,
				Args:         command.Args,
				ProcessGroup: command.ProcessGroup,
				Cwd:          spec.Cwd,
				Env:          spec.Env,
			},
		)
		if err != nil {
			// Only a cancelled setup caused by an explicit kill has a signal
			// result; executable and cwd failures retain their classification.
			if errors.Is(err, context.Canceled) && errors.Is(context.Cause(ctx), errTaskKilled) {
				signal := "SIGKILL"
				return runnerwire.Encode(&runnerwire.SpawnExit{SpawnID: spec.ID, Signal: &signal})
			}
			return rawSpawnFailure(spec.ID, err)
		}
		child = taskExecution{
			input:  started.Input,
			output: started.Output,
			cancel: started.Cancel,
			signal: started.Signal,
			wait:   func(ctx context.Context) (process.Exit, *string) { return started.Wait(ctx), nil },
		}
	case *ShellCommand:
		return t.runShell(spec, entry, command)
	}
	return t.executeTask(spec, entry, child, nil, nil, nil, nil)
}

// rawSpawnFailure preserves the process package's classified start failure.
func rawSpawnFailure(id string, err error) ([]byte, error) {
	var failure *process.SpawnFailure
	if errors.As(err, &failure) {
		return runnerwire.Encode(
			&runnerwire.SpawnExit{
				SpawnID:    id,
				SpawnError: &runnerwire.SpawnError{Kind: failure.Kind, Detail: &failure.Message},
			},
		)
	}
	return nil, err
}

// executionContext pins the selected manifest and registers its authority before shell start.
func (t *Table) executionContext(
	ctx context.Context,
	job string,
	declared DeclaredCommands,
	edits commandwire.EditContext,
	env map[string]string,
) (*ExecutionContext, *process.JobCommands, error) {
	commands := t.config.Commands
	if commands == nil {
		return nil, nil, errors.New("shell command dispatcher is unavailable")
	}
	phase, manifest, err := commands.Installation.Wait(ctx)
	if err != nil {
		return nil, nil, err
	}
	if phase != ManifestReady || manifest.Hash != declared.ManifestHash {
		return nil, nil, errors.New("job manifest is not installed")
	}
	execution, err := NewExecutionContext(
		ctx,
		job,
		declared.Context,
		manifest,
		edits,
		commands.Connection,
		commands.Paths,
	)
	if err != nil {
		return nil, nil, err
	}
	success := false
	defer func() {
		if !success {
			_ = execution.Close(context.WithoutCancel(ctx))
		}
	}()
	leases, err := Leases(ctx, manifest, commands.Services)
	if err != nil {
		return nil, nil, err
	}
	if err = commands.Connection.RegisterContext(ctx, execution, leases); err != nil {
		for _, lease := range leases {
			lease.Release()
		}
		return nil, nil, err
	}
	if err := executionEnvironment(execution, commands, env); err != nil {
		return nil, nil, err
	}
	roots := make([]string, 0, len(manifest.Roots))
	for name := range manifest.Roots {
		roots = append(roots, name)
	}
	success = true
	return execution, &process.JobCommands{Context: execution.ID, Roots: roots, Handler: commands.Dispatcher}, nil
}

// downloadInput sends EOF even when the finite input pipe was refused.
func (t *Table) downloadInput(
	ctx context.Context,
	reference *runnerwire.PipeRef,
	input chan<- process.Input,
) (err error) {
	defer func() {
		select {
		case <-ctx.Done():
		case input <- process.Input{}:
		}
	}()
	body, err := t.config.Pipes.Open(ctx, reference.URL)
	if err != nil {
		return err
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() { _ = body.Close() }()
	for {
		bytes := make([]byte, 65536)
		n, err := body.Read(bytes)
		if n > 0 {
			select {
			case <-ctx.Done():
				return ctx.Err()
			case input <- process.Input{Bytes: bytes[:n]}:
			}
		}
		if errors.Is(err, io.EOF) {
			return nil
		}
		if err != nil {
			return err
		}
	}
}

// chunkSource bridges a bounded job-output queue to demand-driven pipe uploads.
type chunkSource struct{ chunks <-chan []byte }

func (s *chunkSource) Next(ctx context.Context) ([]byte, error) {
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case bytes, ok := <-s.chunks:
		if !ok {
			return nil, io.EOF
		}
		return bytes, nil
	}
}

func (t *Table) runShell(spec TaskSpec, entry *taskEntry, command *ShellCommand) (frame []byte, err error) {
	ctx := entry.lifetime
	directory, err := t.config.Directories.Create(ctx, spec.ID)
	if err != nil {
		return nil, err
	}
	defer func() {
		if finishErr := directory.Finish(context.WithoutCancel(ctx)); err == nil {
			err = finishErr
		}
	}()
	if spec.Env == nil {
		spec.Env = make(map[string]string)
	}
	spec.Env["TMPDIR"] = directory.Scratch
	spec.Env["TEMP"] = directory.Scratch
	spec.Env["DEMI_JOB_ID"] = spec.ID
	edits := commandwire.EditContext{
		Directory: filepath.Join(directory.Path, "changes"),
		Lock:      filepath.Join(t.config.Directories.Root(), "edits.lock"),
	}
	recorder, err := cmdsdk.NewRecorder(ctx, edits)
	if err != nil {
		slog.Warn("edit recording failed", "error", err)
		recorder = nil
	}
	var declared *process.JobCommands
	if command.Commands != nil {
		var execution *ExecutionContext
		execution, declared, err = t.executionContext(ctx, spec.ID, *command.Commands, edits, spec.Env)
		if err != nil {
			return nil, err
		}
		defer func() {
			if closeErr := execution.Close(context.WithoutCancel(ctx)); closeErr != nil {
				slog.Warn("execution context cleanup failed", "error", closeErr)
			}
		}()
	}
	shell, err := t.config.Shell.Start(
		ctx,
		process.JobStart{
			Script:   command.Script,
			Cwd:      spec.Cwd,
			Env:      spec.Env,
			Live:     command.Stdin == nil,
			Commands: declared,
			Edits:    recorder,
		},
	)
	if err != nil {
		return nil, err
	}
	child := taskExecution{
		input:  shell.Input(),
		output: shell.Output(),
		cancel: shell.Cancel,
		signal: func(_ context.Context, signal runnerwire.Signal) error { return shell.Signal(signal) },
		wait:   shell.Wait,
	}
	return t.executeTask(spec, entry, child, directory, recorder, command.Stdin, command.Stdout)
}

// executeTask joins pipe workers before child cleanup; shell authority and directory cleanup remain with runShell.
func (t *Table) executeTask(
	spec TaskSpec,
	entry *taskEntry,
	child taskExecution,
	directory *Directory,
	recorder *cmdsdk.Recorder,
	stdin, stdout *runnerwire.PipeRef,
) (frame []byte, err error) {
	ctx := entry.lifetime
	defer func() {
		child.cancel()
		child.wait(context.WithoutCancel(ctx))
	}()
	pipeCtx, stopPipes := context.WithCancel(ctx)
	inputCtx, stopInput := context.WithCancel(pipeCtx)
	var pipes sync.WaitGroup
	defer func() {
		stopInput()
		stopPipes()
		pipes.Wait()
	}()
	if stdin != nil {
		pipes.Add(1)
		go t.pipeInput(inputCtx, stdin, child.input, &pipes)
	}
	uploads, uploadEnded := t.startOutputUpload(pipeCtx, stdout, &pipes)
	out := &outputView{stream: runnerwire.Stdout}
	stderr := &outputView{stream: runnerwire.Stderr}
	views := []*outputView{out, stderr}
	timer := time.NewTimer(time.Hour)
	timer.Stop()
	defer timer.Stop()
	followed, failure := t.taskOutput(ctx, timer, taskOutputOptions{
		spec: spec, entry: entry, child: child, directory: directory,
		out: out, stderr: stderr, views: views, pipeCtx: pipeCtx, uploads: uploads, uploadEnded: uploadEnded,
	})
	if directory != nil {
		if err := t.finishTaskOutput(t.lifetime, spec.ID, views, followed); err != nil {
			return nil, err
		}
	}
	exit, cwd := child.wait(context.WithoutCancel(ctx))
	if uploads != nil {
		close(uploads)
	}
	stopInput()
	joinTaskPipes(ctx, &pipes, stopPipes)
	return t.taskExit(
		ctx,
		spec.ID,
		taskExitOptions{
			exit:      exit,
			cwd:       cwd,
			failure:   failure,
			directory: directory,
			recorder:  recorder,
			out:       out,
			stderr:    stderr,
		},
	)
}

func (t *Table) pipeInput(
	ctx context.Context,
	stdin *runnerwire.PipeRef,
	input chan<- process.Input,
	pipes *sync.WaitGroup,
) {
	defer pipes.Done()
	result := t.downloadInput(ctx, stdin, input)
	if err := process.ReportPipe(
		t.lifetime,
		t.config.Output,
		stdin.ID,
		result,
	); err != nil &&
		t.lifetime.Err() == nil {
		slog.Warn("pipe reporting failed", "error", err)
	}
}

func (t *Table) pipeOutput(
	ctx context.Context,
	stdout *runnerwire.PipeRef,
	uploads <-chan []byte,
	uploadDone chan<- struct{},
	pipes *sync.WaitGroup,
) {
	defer pipes.Done()
	source := &chunkSource{chunks: uploads}
	body := newInvocationBody(ctx, cmdsdk.NewInput(source))
	result := t.config.Pipes.Put(ctx, stdout.URL, body)
	close(uploadDone)
	if err := process.ReportPipe(
		t.lifetime,
		t.config.Output,
		stdout.ID,
		result,
	); err != nil &&
		t.lifetime.Err() == nil {
		slog.Warn("pipe reporting failed", "error", err)
	}
}

type taskOutputOptions struct {
	spec        TaskSpec
	entry       *taskEntry
	child       taskExecution
	directory   *Directory
	out, stderr *outputView
	views       []*outputView
	pipeCtx     context.Context
	uploads     chan<- []byte
	uploadEnded <-chan struct{}
}

// taskOutput drains child output even after cancellation so the child owner can finish.
func (t *Table) taskOutput(
	ctx context.Context,
	timer *time.Timer,
	options taskOutputOptions,
) (followed bool, failure error) {
	followed = false
	var pending *process.Input
	incoming := options.entry.input
	cancelled := ctx.Done()
	output := options.child.output
	for output != nil {
		var due <-chan time.Time
		if options.directory != nil {
			due = t.outputDeadline(options, followed, timer, &failure)
		}
		var sending chan<- process.Input
		var value process.Input
		if pending != nil {
			sending = options.child.input
			value = *pending
		}
		select {
		case <-cancelled:
			options.child.cancel()
			cancelled = nil
		case signal := <-options.entry.signals:
			signalTask(ctx, options.child, signal, &failure)
		case input := <-incoming:
			pending = &input
			incoming = nil
		case sending <- value:
			if pending.Bytes != nil {
				incoming = options.entry.input
			}
			pending = nil
		case <-due:
		case <-options.entry.following:
			t.mu.Lock()
			followed = options.entry.follow
			t.mu.Unlock()
			if options.directory != nil && followed {
				t.flushFollowedOutput(options, &failure)
			}
		case chunk, ok := <-output:
			if !ok {
				output = nil
				break
			}
			frame, err := t.taskOutputFrame(ctx, options, chunk)
			if err != nil {
				failure = err
				options.child.cancel()
				output = nil
				break
			}
			t.deliverTaskOutput(ctx, options, chunk, frame)
		}
		timer.Stop()
	}
	return followed, failure
}

// joinTaskPipes gives uploads the Rust owner’s grace period and still joins on cancellation.
func joinTaskPipes(ctx context.Context, pipes *sync.WaitGroup, stopPipes context.CancelFunc) {
	joined := make(chan struct{})
	go func() {
		pipes.Wait()
		close(joined)
	}()
	grace := time.NewTimer(30 * time.Second)
	select {
	case <-joined:
	case <-ctx.Done():
		stopPipes()
		<-joined
	case <-grace.C:
		stopPipes()
		<-joined
	}
	grace.Stop()
}

type taskExitOptions struct {
	exit        process.Exit
	cwd         *string
	failure     error
	directory   *Directory
	recorder    *cmdsdk.Recorder
	out, stderr *outputView
}

// taskExit preserves the child status when a runner failure can only be logged.
func (t *Table) taskExit(ctx context.Context, id string, options taskExitOptions) ([]byte, error) {
	if options.failure == nil && options.exit.Error != nil {
		options.failure = errors.New(*options.exit.Error)
	}
	var spawnError *runnerwire.SpawnError
	if options.failure != nil {
		if options.exit.Code == nil && options.exit.Signal == nil {
			reason := options.failure.Error()
			spawnError = &runnerwire.SpawnError{Kind: runnerwire.SpawnErrorKindOther, Detail: &reason}
		} else {
			slog.Warn("task ended with a failure of the runner's", "error", options.failure)
		}
	}
	if options.directory != nil {
		files, truncated := finishEdits(context.WithoutCancel(ctx), options.recorder)
		return runnerwire.Encode(
			&runnerwire.JobExit{
				JobID:      id,
				ExitCode:   options.exit.Code,
				Signal:     options.exit.Signal,
				SpawnError: spawnError,
				CWD:        options.cwd,
				Output: &runnerwire.OutputLengths{
					StdoutBytes: options.out.length,
					StderrBytes: options.stderr.length,
				},
				Files:          files,
				FilesTruncated: truncated,
			},
		)
	}
	return runnerwire.Encode(
		&runnerwire.SpawnExit{
			SpawnID:    id,
			ExitCode:   options.exit.Code,
			Signal:     options.exit.Signal,
			SpawnError: spawnError,
		},
	)
}

func executionEnvironment(execution *ExecutionContext, commands *Commands, env map[string]string) error {
	var path *string
	if value, ok := env["PATH"]; ok {
		path = &value
	}
	values, err := execution.Environment(commands.Endpoint, commands.Home, path)
	if err != nil {
		return err
	}
	for key, value := range values {
		env[key] = value
	}
	return nil
}

func (t *Table) taskOutputFrame(
	ctx context.Context,
	options taskOutputOptions,
	chunk process.OutputChunk,
) ([]byte, error) {
	var frame []byte
	var err error
	if options.directory != nil {
		if writeErr := options.directory.Output.Write(
			context.WithoutCancel(ctx),
			chunk.Stream,
			chunk.Bytes,
		); writeErr != nil {
			return nil, writeErr
		}
		view := options.out
		if chunk.Stream == runnerwire.Stderr {
			view = options.stderr
		}
		offset, head := view.write(chunk.Bytes)
		if len(head) > 0 {
			frame, err = runnerwire.Encode(
				&runnerwire.JobOutput{JobID: options.spec.ID, Stream: chunk.Stream, Offset: offset, Bytes: head},
			)
		}
	} else {
		frame, err = runnerwire.Encode(
			&runnerwire.SpawnOutput{SpawnID: options.spec.ID, Stream: chunk.Stream, Bytes: chunk.Bytes},
		)
	}
	if err != nil {
		return nil, err
	}
	return frame, nil
}

func (t *Table) deliverTaskOutput(
	ctx context.Context,
	options taskOutputOptions,
	chunk process.OutputChunk,
	frame []byte,
) {
	if frame != nil {
		select {
		case <-ctx.Done():
			options.child.cancel()
		case t.config.Output <- frame:
		}
	}
	if chunk.Stream == runnerwire.Stdout && options.uploads != nil {
		select {
		case <-options.pipeCtx.Done():
			options.child.cancel()
		case <-options.uploadEnded:
		case options.uploads <- chunk.Bytes:
		}
	}
}

func (t *Table) outputDeadline(
	options taskOutputOptions,
	followed bool,
	timer *time.Timer,
	failure *error,
) <-chan time.Time {
	var at time.Time
	for _, view := range options.views {
		if err := view.beyond(options.spec.ID, followed, false, t.config.Output); err != nil {
			*failure = err
			options.child.cancel()
		}
		next := view.due(followed)
		if !next.IsZero() && (at.IsZero() || next.Before(at)) {
			at = next
		}
	}
	if !at.IsZero() {
		timer.Reset(max(0, time.Until(at)))
		return timer.C
	}
	return nil
}

func (t *Table) flushFollowedOutput(options taskOutputOptions, failure *error) {
	for _, view := range options.views {
		if err := view.beyond(options.spec.ID, true, true, t.config.Output); err != nil {
			*failure = err
			options.child.cancel()
		}
	}
}

func signalTask(ctx context.Context, child taskExecution, signal runnerwire.Signal, failure *error) {
	if err := child.signal(ctx, signal); err != nil {
		*failure = err
		child.cancel()
	}
}

func (t *Table) finishTaskOutput(ctx context.Context, id string, views []*outputView, followed bool) error {
	for _, view := range views {
		if err := view.last(ctx, id, followed, t.config.Output); err != nil {
			return err
		}
	}
	return nil
}

// startOutputUpload registers the output worker before returning its queue and completion signal.
func (t *Table) startOutputUpload(
	ctx context.Context,
	stdout *runnerwire.PipeRef,
	pipes *sync.WaitGroup,
) (chan []byte, <-chan struct{}) {
	if stdout == nil {
		return nil, nil
	}
	uploads := make(chan []byte, 4)
	uploadDone := make(chan struct{})
	pipes.Add(1)
	go t.pipeOutput(ctx, stdout, uploads, uploadDone, pipes)
	return uploads, uploadDone
}
