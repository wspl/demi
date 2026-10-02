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
	var directory *Directory
	var recorder *cmdsdk.Recorder
	var stdin, stdout *runnerwire.PipeRef
	switch command := spec.Command.(type) {
	case *ProcessCommand:
		started, err := process.Spawn(ctx, process.SpawnOptions{Command: command.Command, Args: command.Args, ProcessGroup: command.ProcessGroup, Cwd: spec.Cwd, Env: spec.Env})
		if err != nil {
			return rawSpawnFailure(spec.ID, err)
		}
		child = taskExecution{input: started.Input, output: started.Output, cancel: started.Cancel, signal: started.Signal, wait: func(ctx context.Context) (process.Exit, *string) { return started.Wait(ctx), nil }}
	case *ShellCommand:
		directory, err = t.config.Directories.Create(ctx, spec.ID)
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
		edits := commandwire.EditContext{Directory: filepath.Join(directory.Path, "changes"), Lock: filepath.Join(t.config.Directories.Root(), "edits.lock")}
		recorder, err = cmdsdk.NewRecorder(ctx, edits)
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
		shell, err := t.config.Shell.Start(ctx, process.JobStart{Script: command.Script, Cwd: spec.Cwd, Env: spec.Env, Live: command.Stdin == nil, Commands: declared, Edits: recorder})
		if err != nil {
			return nil, err
		}
		child = taskExecution{input: shell.Input(), output: shell.Output(), cancel: shell.Cancel, signal: func(_ context.Context, signal runnerwire.Signal) error { return shell.Signal(signal) }, wait: shell.Wait}
		stdin, stdout = command.Stdin, command.Stdout
	}
	defer func() { child.cancel(); child.wait(context.WithoutCancel(ctx)) }()
	pipeCtx, stopPipes := context.WithCancel(ctx)
	inputCtx, stopInput := context.WithCancel(pipeCtx)
	var pipes sync.WaitGroup
	defer func() { stopInput(); stopPipes(); pipes.Wait() }()
	if stdin != nil {
		pipes.Add(1)
		go func() {
			defer pipes.Done()
			result := t.downloadInput(inputCtx, stdin, child.input)
			if err := process.ReportPipe(t.lifetime, t.config.Output, stdin.ID, result); err != nil && t.lifetime.Err() == nil {
				slog.Warn("pipe reporting failed", "error", err)
			}
		}()
	}
	var uploads chan []byte
	var uploadEnded <-chan struct{}
	if stdout != nil {
		uploads = make(chan []byte, 4)
		uploadDone := make(chan struct{})
		uploadEnded = uploadDone
		pipes.Add(1)
		go func() {
			defer pipes.Done()
			source := &chunkSource{chunks: uploads}
			body := newInvocationBody(pipeCtx, cmdsdk.NewInput(source))
			result := t.config.Pipes.Put(pipeCtx, stdout.URL, body)
			close(uploadDone)
			if err := process.ReportPipe(t.lifetime, t.config.Output, stdout.ID, result); err != nil && t.lifetime.Err() == nil {
				slog.Warn("pipe reporting failed", "error", err)
			}
		}()
	}
	out := &outputView{stream: runnerwire.Stdout}
	stderr := &outputView{stream: runnerwire.Stderr}
	views := []*outputView{out, stderr}
	followed := false
	timer := time.NewTimer(time.Hour)
	timer.Stop()
	defer timer.Stop()
	var failure error
	var pending *process.Input
	incoming := entry.input
	cancelled := ctx.Done()
	output := child.output
	for output != nil {
		var due <-chan time.Time
		if directory != nil {
			var at time.Time
			for _, view := range views {
				if err := view.beyond(spec.ID, followed, false, t.config.Output); err != nil {
					failure = err
					child.cancel()
				}
				next := view.due(followed)
				if !next.IsZero() && (at.IsZero() || next.Before(at)) {
					at = next
				}
			}
			if !at.IsZero() {
				timer.Reset(max(0, time.Until(at)))
				due = timer.C
			}
		}
		var sending chan<- process.Input
		var value process.Input
		if pending != nil {
			sending = child.input
			value = *pending
		}
		select {
		case <-cancelled:
			child.cancel()
			cancelled = nil
		case signal := <-entry.signals:
			if err := child.signal(ctx, signal); err != nil {
				failure = err
				child.cancel()
			}
		case input := <-incoming:
			pending = &input
			incoming = nil
		case sending <- value:
			if pending.Bytes != nil {
				incoming = entry.input
			}
			pending = nil
		case <-due:
		case <-entry.following:
			t.mu.Lock()
			followed = entry.follow
			t.mu.Unlock()
			if directory != nil && followed {
				for _, view := range views {
					if err := view.beyond(spec.ID, true, true, t.config.Output); err != nil {
						failure = err
						child.cancel()
					}
				}
			}
		case chunk, ok := <-output:
			if !ok {
				output = nil
				break
			}
			var frame []byte
			if directory != nil {
				if writeErr := directory.Output.Write(context.WithoutCancel(ctx), chunk.Stream, chunk.Bytes); writeErr != nil {
					failure = writeErr
					child.cancel()
					output = nil
					break
				}
				view := out
				if chunk.Stream == runnerwire.Stderr {
					view = stderr
				}
				offset, head := view.write(chunk.Bytes)
				if len(head) > 0 {
					frame, err = runnerwire.Encode(&runnerwire.JobOutput{JobID: spec.ID, Stream: chunk.Stream, Offset: offset, Bytes: head})
				}
			} else {
				frame, err = runnerwire.Encode(&runnerwire.SpawnOutput{SpawnID: spec.ID, Stream: chunk.Stream, Bytes: chunk.Bytes})
			}
			if err != nil {
				failure = err
				child.cancel()
				output = nil
				break
			}
			if frame != nil {
				select {
				case <-ctx.Done():
					child.cancel()
				case t.config.Output <- frame:
				}
			}
			if chunk.Stream == runnerwire.Stdout && uploads != nil {
				select {
				case <-pipeCtx.Done():
					child.cancel()
				case <-uploadEnded:
				case uploads <- chunk.Bytes:
				}
			}
		}
		timer.Stop()
	}
	if directory != nil {
		for _, view := range views {
			if err := view.last(t.lifetime, spec.ID, followed, t.config.Output); err != nil {
				return nil, err
			}
		}
	}
	exit, cwd := child.wait(context.WithoutCancel(ctx))
	if uploads != nil {
		close(uploads)
	}
	stopInput()
	joined := make(chan struct{})
	go func() { pipes.Wait(); close(joined) }()
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
	if failure == nil && exit.Error != nil {
		failure = errors.New(*exit.Error)
	}
	var spawnError *runnerwire.SpawnError
	if failure != nil {
		if exit.Code == nil && exit.Signal == nil {
			reason := failure.Error()
			spawnError = &runnerwire.SpawnError{Kind: runnerwire.SpawnErrorKindOther, Detail: &reason}
		} else {
			slog.Warn("task ended with a failure of the runner's", "error", failure)
		}
	}
	if directory != nil {
		files, truncated := finishEdits(context.WithoutCancel(ctx), recorder)
		return runnerwire.Encode(&runnerwire.JobExit{JobID: spec.ID, ExitCode: exit.Code, Signal: exit.Signal, SpawnError: spawnError, CWD: cwd, Output: &runnerwire.OutputLengths{StdoutBytes: out.length, StderrBytes: stderr.length}, Files: files, FilesTruncated: truncated})
	}
	return runnerwire.Encode(&runnerwire.SpawnExit{SpawnID: spec.ID, ExitCode: exit.Code, Signal: exit.Signal, SpawnError: spawnError})
}

// rawSpawnFailure preserves the process package's classified start failure.
func rawSpawnFailure(id string, err error) ([]byte, error) {
	var failure *process.SpawnFailure
	if errors.As(err, &failure) {
		return runnerwire.Encode(&runnerwire.SpawnExit{SpawnID: id, SpawnError: &runnerwire.SpawnError{Kind: failure.Kind, Detail: &failure.Message}})
	}
	return nil, err
}

// executionContext pins the selected manifest and registers its authority before shell start.
func (t *Table) executionContext(ctx context.Context, job string, declared DeclaredCommands, edits commandwire.EditContext, env map[string]string) (*ExecutionContext, *process.JobCommands, error) {
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
	execution, err := NewExecutionContext(ctx, job, declared.Context, manifest, edits, commands.Connection, commands.Paths)
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
	var path *string
	if value, ok := env["PATH"]; ok {
		path = &value
	}
	values, err := execution.Environment(commands.Endpoint, commands.Home, path)
	if err != nil {
		return nil, nil, err
	}
	for key, value := range values {
		env[key] = value
	}
	roots := make([]string, 0, len(manifest.Roots))
	for name := range manifest.Roots {
		roots = append(roots, name)
	}
	success = true
	return execution, &process.JobCommands{Context: execution.ID, Roots: roots, Handler: commands.Dispatcher}, nil
}

// downloadInput sends EOF even when the finite input pipe was refused.
func (t *Table) downloadInput(ctx context.Context, reference *runnerwire.PipeRef, input chan<- process.Input) (err error) {
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
	defer func() { _ = body.Close() }() // Cleanup follows the operation result; cancellation may already have closed it.
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
