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
	"github.com/wspl/demi/internal/runnerwire"
)

type job struct {
	ctx    context.Context
	cancel context.CancelFunc
	input  chan process.Input
	output chan process.OutputChunk
	done   chan struct{}
	mu     sync.Mutex // Protects the first requested signal.
	signal string
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
	j := &job{ctx: ctx, cancel: cancel, input: make(chan process.Input, 4), output: make(chan process.OutputChunk, 4), done: make(chan struct{})}
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
		defer func() { _ = files[1].Close() }() // Cleanup also runs after cancellation closes the file.
		for {
			select {
			case <-inputCtx.Done():
				return
			case input, ok := <-j.input:
				if !ok || input.Bytes == nil {
					return
				}
				if _, err := files[1].Write(input.Bytes); err != nil {
					if inputCtx.Err() == nil && !errors.Is(err, syscall.EPIPE) && !errors.Is(err, os.ErrClosed) {
						inputError = err
					}
					return
				}
			}
		}
	}()
	var drains sync.WaitGroup
	var outputErrors [2]error
	for index, stream := range []runnerwire.OutputStream{runnerwire.Stdout, runnerwire.Stderr} {
		file := files[2+index*2]
		drains.Go(func() {
			buffer := make([]byte, 64*1024)
			for {
				n, err := file.Read(buffer)
				if n > 0 {
					chunk := process.OutputChunk{Stream: stream, Bytes: append([]byte(nil), buffer[:n]...)}
					select {
					case j.output <- chunk:
					case <-j.ctx.Done():
						return
					}
				}
				if err != nil {
					if !errors.Is(err, io.EOF) && j.ctx.Err() == nil {
						outputErrors[index] = err
					}
					return
				}
			}
		})
	}
	result, err := Execute(j.ctx, start.Script, Options{Login: true, Cwd: start.Cwd, Env: env, Stdin: files[0], Stdout: files[3], Stderr: files[5], Commands: start.Commands, Edits: start.Edits, Observe: observe, Interrupt: func() {
		for _, file := range files {
			_ = file.Close()
		}
	}})
	stopInput()
	_ = files[0].Close()
	_ = files[1].Close()
	<-inputDone
	_ = files[3].Close()
	_ = files[5].Close()
	drains.Wait()
	close(j.output)
	if j.ctx.Err() != nil {
		j.mu.Lock()
		signal := j.signal
		j.mu.Unlock()
		if signal == "" {
			signal = "SIGKILL"
		}
		j.exit.Signal = &signal
	} else if err != nil {
		message := err.Error()
		j.exit.Error = &message
	} else {
		code := int32(result.Code)
		j.exit.Code = &code
		j.cwd = &result.Cwd
	}
	if j.ctx.Err() == nil {
		if failure := errors.Join(err, inputError, outputErrors[0], outputErrors[1]); failure != nil {
			message := failure.Error()
			j.exit.Error = &message
		}
	}
}
func (j *job) Input() chan<- process.Input        { return j.input }
func (j *job) Output() <-chan process.OutputChunk { return j.output }
func (j *job) Cancel()                            { j.cancel() }
func (j *job) IsCancelled() bool {
	select {
	case <-j.done:
		return j.exit.Signal != nil
	default:
		return j.ctx.Err() != nil
	}
}
func (j *job) Signal(signal runnerwire.Signal) error {
	switch signal {
	case runnerwire.SignalInterrupt, runnerwire.SignalTerminate, runnerwire.SignalKill, runnerwire.SignalHangup, runnerwire.SignalQuit:
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
func (j *job) Wait(ctx context.Context) (process.Exit, *string) {
	select {
	case <-j.done:
	case <-ctx.Done():
		j.cancel()
		<-j.done
	}
	return j.exit, j.cwd
}
