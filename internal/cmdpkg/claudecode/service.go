package claudecode

import (
	"context"
	"errors"
	"fmt"
	"io"
	"sync/atomic"

	"github.com/wspl/demi/internal/cmdpkg/claudecode/claudecodeop"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

const maxInputBytes = 64 * 1024

// Serve validates the launch argument and serves the command wire on process stdio.
// The executable must exit when Serve returns.
func Serve(ctx context.Context, args []string) error {
	if _, err := cmdsdk.ParseLaunch(args); err != nil {
		return err
	}
	return cmdsdk.ServeStdio(ctx, &service{})
}

type service struct {
	artifacts atomic.Pointer[cmdsdk.Artifacts]
}

func (s *service) SetArtifacts(a *cmdsdk.Artifacts) { s.artifacts.Store(a) }
func (*service) Operations() []string {
	operations := claudecodeop.Operations()
	result := make([]string, len(operations))
	for i, op := range operations {
		result[i] = string(op)
	}
	return result
}

func (s *service) Invoke(ctx context.Context, invocation cmdsdk.InvocationContext[commandwire.Invocation]) (commandwire.Completion, error) {
	operation, err := claudecodeop.ParseOperation(invocation.Request.Operation)
	if err != nil {
		return commandwire.Completion{}, fmt.Errorf("unknown operation: %s", invocation.Request.Operation)
	}
	body, err := s.document(ctx, operation, invocation)
	completion := commandwire.Completion{}
	if err != nil {
		if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
			return completion, err
		}
		code := claudecodeop.InstallFailed
		var failure *operationError
		if errors.As(err, &failure) {
			code = failure.code
		}
		message := err.Error()
		body, err = (claudecodeop.Failed{Failure: claudecodeop.Failure{Code: code, Message: message}}).MarshalJSON()
		if err != nil {
			return completion, err
		}
		completion = commandwire.Completion{ExitCode: 1, Error: &commandwire.CommandError{Code: string(code), Message: message}}
	}
	return completion, invocation.Output.Stdout(ctx, append(body, '\n'))
}

func (s *service) document(ctx context.Context, operation claudecodeop.Operation, invocation cmdsdk.InvocationContext[commandwire.Invocation]) ([]byte, error) {
	artifacts := s.artifacts.Load()
	if artifacts == nil {
		return nil, &operationError{claudecodeop.InstallFailed, errors.New("demi-claude-code has no artifacts source")}
	}
	switch operation {
	case claudecodeop.OperationEnsure:
		input, err := readInput(ctx, invocation.Input)
		if err != nil {
			return nil, err
		}
		release, err := parseRelease(input)
		if err != nil {
			return nil, err
		}
		installed, err := ensure(ctx, artifacts, invocation.Request.InvocationID, release)
		if err != nil {
			return nil, err
		}
		return (claudecodeop.Ensured{Installed: installed}).MarshalJSON()
	case claudecodeop.OperationStatus:
		result, err := status(ctx, artifacts)
		if err != nil {
			return nil, err
		}
		return (claudecodeop.StatusDone{Status: result}).MarshalJSON()
	}
	return nil, fmt.Errorf("unknown operation: %s", operation)
}

func readInput(ctx context.Context, input *cmdsdk.Input) ([]byte, error) {
	var body []byte
	for {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		chunk, err := input.Next(ctx)
		if errors.Is(err, io.EOF) {
			return body, nil
		}
		if err != nil {
			return nil, &operationError{claudecodeop.InvalidRelease, err}
		}
		if len(chunk) > maxInputBytes-len(body) {
			return nil, &operationError{claudecodeop.InvalidRelease, fmt.Errorf("input exceeds %d bytes", maxInputBytes)}
		}
		body = append(body, chunk...)
	}
}
