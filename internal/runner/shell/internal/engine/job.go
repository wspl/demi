package engine

import (
	"context"
	"errors"
	"io"
	"maps"
	"os"
	"sync"
	"syscall"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerproto"
)

type job struct {
	ctx    context.Context
	cancel context.CancelFunc
	input  chan process.Input
	output chan process.OutputChunk
	done   chan struct{}
	mu     sync.Mutex // Protects the first requested signal.
	signal string
	err    error
	exit   process.Exit
	cwd    *string
}

// StartJob owns three pipe pairs and joins input, output and interpreter work.
func StartJob(ctx context.Context, start process.JobStart, observe Observer) (process.ShellJob, error) {
	ctx, cancel := context.WithCancel(ctx)
	var files []*os.File
	success := false
	defer func() {
		if !success {
			cancel()
			for _, f := range files {
				_ = f.Close()
			}
		}
	}()
	for range 3 {
		pair, err := cmdsdk.Retry(ctx, func() ([2]*os.File, error) {
			r, w, err := os.Pipe()
			return [2]*os.File{r, w}, err
		})
		if err != nil {
			return nil, err
		}
		files = append(files, pair[:]...)
	}
	env := maps.Clone(start.Env)
	if env == nil {
		env = map[string]string{}
	}
	if start.Commands != nil {
		env[process.ContextEnv] = start.Commands.Context
	}
	delete(env, process.LiveInputEnv)
	if start.Live {
		reference, err := process.LiveReference(files[0])
		if err != nil {
			return nil, err
		}
		env[process.LiveInputEnv] = reference
	}
	j := &job{
		ctx:    ctx,
		cancel: cancel,
		input:  make(chan process.Input, 4),
		output: make(chan process.OutputChunk, 4),
		done:   make(chan struct{}),
	}
	go j.run(start, env, files, observe)
	success = true
	return j, nil
}

func (j *job) run(start process.JobStart, env map[string]string, files []*os.File, observe Observer) {
	defer close(j.done)
	defer j.cancel()
	defer func() {
		for _, f := range files {
			_ = f.Close()
		}
	}()
	inputCtx, stopInput := context.WithCancel(j.ctx)
	inputDone := make(chan struct{})
	var inputError error
	go func() {
		defer close(inputDone)
		defer func() {
			_ = files[1].Close()
		}() // Cleanup also runs after cancellation closes the file.
		inputError = j.writeInput(inputCtx, files[1])
	}()
	var drains sync.WaitGroup
	var outputErrors [2]error
	for index, stream := range []runnerproto.OutputStream{runnerproto.Stdout, runnerproto.Stderr} {
		file := files[2+index*2]
		drains.Go(func() {
			outputErrors[index] = j.readOutput(j.ctx, file, stream)
		})
	}
	result, err := Execute(
		j.ctx,
		start.Script,
		Options{
			Login:    true,
			Cwd:      start.Cwd,
			Env:      env,
			Stdin:    files[0],
			Stdout:   files[3],
			Stderr:   files[5],
			Commands: start.Commands,
			Edits:    start.Edits,
			Observe:  observe,
			Interrupt: func() {
				for _, file := range files {
					_ = file.Close()
				}
			},
		},
	)
	stopInput()
	_ = files[0].Close()
	_ = files[1].Close()
	<-inputDone
	_ = files[3].Close()
	_ = files[5].Close()
	drains.Wait()
	close(j.output)
	j.finish(result, err)
	if j.ctx.Err() == nil {
		if failure := errors.Join(err, inputError, outputErrors[0], outputErrors[1]); failure != nil {
			j.err = failure
		}
	}
}

// Input exposes the bounded input channel consumed by the job.
func (j *job) Input() chan<- process.Input {
	return j.input
}

// Output exposes stdout and stderr chunks until all drains finish.
func (j *job) Output() <-chan process.OutputChunk {
	return j.output
}

// Cancel requests cancellation of the interpreter and its owned work.
func (j *job) Cancel() {
	j.cancel()
}

// IsCancelled reports cancellation while running or the final signal after completion.
func (j *job) IsCancelled() bool {
	select {
	case <-j.done:
		return j.exit.Signal != nil
	default:
		return j.ctx.Err() != nil
	}
}

// Signal records the first supported cancellation signal and cancels the job.
func (j *job) Signal(signal runnerproto.Signal) error {
	switch signal {
	case runnerproto.SignalInterrupt,
		runnerproto.SignalTerminate,
		runnerproto.SignalKill,
		runnerproto.SignalHangup,
		runnerproto.SignalQuit:
		j.mu.Lock()
		if j.signal == "" && j.ctx.Err() == nil {
			j.signal = string(signal)
		}
		j.mu.Unlock()
		j.cancel()
		return nil
	default:
		return errors.New("unsupported shell job signal")
	}
}

// Wait joins all job work, cancelling it if the waiting context ends.
func (j *job) Wait(ctx context.Context) (process.Exit, *string, error) {
	select {
	case <-j.done:
	case <-ctx.Done():
		j.cancel()
		<-j.done
	}
	return j.exit, j.cwd, j.err
}

func (j *job) writeInput(ctx context.Context, file *os.File) error {
	for {
		select {
		case <-ctx.Done():
			return nil
		case input, ok := <-j.input:
			if !ok || input.Bytes == nil {
				return nil
			}
			if _, err := file.Write(input.Bytes); err != nil {
				if ctx.Err() == nil && !errors.Is(err, syscall.EPIPE) && !errors.Is(err, os.ErrClosed) {
					return err
				}
				return nil
			}
		}
	}
}

func (j *job) readOutput(ctx context.Context, file *os.File, stream runnerproto.OutputStream) error {
	buffer := make([]byte, 64*1024)
	for {
		n, err := file.Read(buffer)
		if n > 0 {
			chunk := process.OutputChunk{Stream: stream, Bytes: append([]byte(nil), buffer[:n]...)}
			select {
			case j.output <- chunk:
			case <-ctx.Done():
				return nil
			}
		}
		if err != nil {
			if !errors.Is(err, io.EOF) && ctx.Err() == nil {
				return err
			}
			return nil
		}
	}
}

// finish publishes the foreground result after all owned IO workers have joined.
func (j *job) finish(result Result, err error) {
	if j.ctx.Err() != nil {
		j.mu.Lock()
		signal := j.signal
		j.mu.Unlock()
		if signal == "" {
			signal = "SIGKILL"
		}
		j.exit.Signal = &signal
	} else if err != nil {
		j.err = err
	} else {
		code := int32(result.Code)
		j.exit.Code = &code
		j.cwd = &result.Cwd
	}
}
