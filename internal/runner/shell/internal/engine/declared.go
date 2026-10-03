package engine

import (
	"context"
	"encoding/hex"
	"errors"
	"io"
	"os"
	"strings"
	"syscall"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/process"
	"mvdan.cc/sh/v3/interp"
)

// declared passes a shell invocation directly to the job's handler with demand-driven input.
func (e *execution) declared(ctx context.Context, args []string) error {
	hc := interp.HandlerCtx(ctx)
	commands := e.options.Commands
	env := make(map[string]string)
	for _, entry := range exported(hc.Env) {
		name, value, _ := strings.Cut(entry, "=")
		env[name] = value
	}
	if options, ok := ctx.Value(execKey{}).(execOptions); ok && options.empty {
		clear(env)
	}
	live := false
	if input, ok := hc.Stdin.(*os.File); ok {
		var err error
		live, err = process.IsLive(input, env)
		if err != nil {
			return err
		}
	}
	raw, err := process.NewRawCommand(commands.Context, args[0], append([]string{}, args[1:]...), live)
	if err != nil {
		return err
	}
	encoded, err := raw.MarshalJSON()
	if err != nil {
		return err
	}
	id, err := uuid.NewRandom()
	if err != nil {
		return err
	}
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	output, records := cmdsdk.OutputChannel(ctx)
	type result struct {
		completion commandwire.Completion
		err        error
	}
	done := make(chan result, 1)
	go func() {
		finished := result{}
		defer func() {
			if recovered := recover(); recovered != nil {
				finished.err = errors.New("shell worker panicked")
			}
			done <- finished
		}()
		completion, err := commands.Handler.Invoke(ctx, cmdsdk.InvocationContext[commandwire.LocalInvocation]{Request: commandwire.LocalInvocation{Operation: process.Raw, InvocationID: hex.EncodeToString(id[:]), Args: encoded, Cwd: hc.Dir, Env: env}, Input: cmdsdk.NewInput(&invocationInput{reader: hc.Stdin, state: hc.Scope().(*interpreterScope)}), Output: output})
		finished = result{completion, err}
	}()
	drain := func(record commandwire.Record) error {
		var writer io.Writer
		var bytes []byte
		switch record := record.(type) {
		case commandwire.Stdout:
			writer = hc.Stdout
			bytes = record
		case commandwire.Stderr:
			writer = hc.Stderr
			bytes = record
		case commandwire.Completed, commandwire.InputPull:
			return errors.New("unexpected local output record")
		}
		_, err := writer.Write(bytes)
		if errors.Is(err, syscall.EPIPE) {
			return interp.ExitStatus(141)
		}
		return err
	}
	for {
		select {
		case record := <-records:
			if err := drain(record); err != nil {
				cancel()
				<-done
				return err
			}
		case finished := <-done:
			for {
				select {
				case record := <-records:
					if err := drain(record); err != nil {
						return err
					}
				default:
					if finished.err != nil {
						return finished.err
					}
					if finished.completion.ExitCode != 0 {
						return interp.ExitStatus(finished.completion.ExitCode)
					}
					return nil
				}
			}
		case <-ctx.Done():
			cancel()
			<-done
			return ctx.Err()
		}
	}
}

type invocationInput struct {
	reader io.Reader
	state  *interpreterScope
}

func (i *invocationInput) Next(ctx context.Context) ([]byte, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if i.reader == nil {
		return nil, io.EOF
	}

	reader := i.reader
	if file, ok := reader.(*os.File); ok && file != nil {
		// Each pull owns its duplicate and cancellation. The shell keeps its
		// original descriptor and any input this pull has not consumed.
		borrowed, err := borrowFile(ctx, file)
		if err != nil {
			return nil, err
		}
		closed := make(chan struct{})
		stop := context.AfterFunc(ctx, func() {
			_ = borrowed.Close()
			close(closed)
		})
		defer func() {
			if !stop() {
				<-closed
			} else {
				_ = borrowed.Close()
			}
		}() // Only the duplicate is closed; a canceled pull never owns stdin.
		reader = borrowed
	}
	i.state.Waiting(1)
	defer i.state.Waiting(-1)
	b := make([]byte, 64*1024)
	n, err := reader.Read(b)
	if ctx.Err() != nil {
		return nil, ctx.Err()
	}
	return b[:n], err
}
