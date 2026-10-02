package engine

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"io"
	"os"
	"strings"
	"syscall"

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
	var id [16]byte
	_, _ = rand.Read(id[:]) // crypto/rand.Read fills the buffer or terminates the process.
	id[6] = id[6]&0x0f | 0x40
	id[8] = id[8]&0x3f | 0x80
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	reader := hc.Stdin
	if file, ok := reader.(*os.File); ok && file != nil {
		borrowed, err := borrowFile(ctx, file)
		if err != nil {
			return err
		}
		defer func() { _ = borrowed.Close() }()
		reader = borrowed
	}
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
		completion, err := commands.Handler.Invoke(ctx, cmdsdk.InvocationContext[commandwire.LocalInvocation]{Request: commandwire.LocalInvocation{Operation: process.Raw, InvocationID: hex.EncodeToString(id[:]), Args: encoded, Cwd: hc.Dir, Env: env}, Input: cmdsdk.NewInput(&invocationInput{reader: reader, state: hc.Scope().(*interpreterScope)}), Output: output})
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
	i.state.Waiting(1)
	defer i.state.Waiting(-1)
	if i.reader == nil {
		return nil, io.EOF
	}

	b := make([]byte, 64*1024)
	n, err := i.reader.Read(b)
	if ctx.Err() != nil {
		return nil, ctx.Err()
	}
	return b[:n], err
}
