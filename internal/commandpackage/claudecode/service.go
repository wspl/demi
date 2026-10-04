package claudecode

import (
	"context"
	"errors"
	"fmt"
	"io"
	"sync/atomic"

	"github.com/wspl/demi/internal/commandpackage/claudecode/claudecodeproto"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
)

const maxInputBytes = 64 * 1024

// Serve validates the launch argument and serves the command wire on process stdio.
// The executable must exit when Serve returns.
func Serve(ctx context.Context, args []string) error {
	if err := commandsdk.CheckLaunch(args); err != nil {
		return err
	}
	return commandsdk.ServeStdio(ctx, &service{})
}

type service struct {
	artifacts atomic.Pointer[commandsdk.Artifacts]
}

// SetArtifacts supplies the runner-owned artifact source.
func (s *service) SetArtifacts(a *commandsdk.Artifacts) {
	s.artifacts.Store(a)
}

// Operations lists the resident service operations.
func (*service) Operations() []string {
	operations := claudecodeproto.Operations()
	result := make([]string, len(operations))
	for i, op := range operations {
		result[i] = string(op)
	}
	return result
}

// Invoke runs an operation and writes its completion document.
func (s *service) Invoke(
	ctx context.Context,
	invocation commandsdk.InvocationContext[commandproto.Invocation],
) (commandproto.Completion, error) {
	operation, err := claudecodeproto.ParseOperation(invocation.Request.Operation)
	if err != nil {
		return commandproto.Completion{}, fmt.Errorf("unknown operation: %s", invocation.Request.Operation)
	}
	body, err := s.document(ctx, operation, invocation)
	completion := commandproto.Completion{}
	if err != nil {
		if errors.Is(err, context.Canceled) || errors.Is(err, context.DeadlineExceeded) {
			return completion, err
		}
		code := claudecodeproto.InstallFailed
		var failure *operationError
		if errors.As(err, &failure) {
			code = failure.code
		}
		message := err.Error()
		body, err = (claudecodeproto.Failed{Failure: claudecodeproto.Failure{Code: code, Message: message}}).MarshalJSON()
		if err != nil {
			return completion, err
		}
		completion = commandproto.Completion{
			ExitCode: 1,
			Error:    &commandproto.CommandError{Code: string(code), Message: message},
		}
	}
	return completion, invocation.Output.Stdout(ctx, append(body, '\n'))
}

func (s *service) document(
	ctx context.Context,
	operation claudecodeproto.Operation,
	invocation commandsdk.InvocationContext[commandproto.Invocation],
) ([]byte, error) {
	artifacts := s.artifacts.Load()
	if artifacts == nil {
		return nil, &operationError{
			claudecodeproto.InstallFailed,
			errors.New("demi-claude-code has no artifacts source"),
		}
	}
	switch operation {
	case claudecodeproto.OperationEnsure:
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
		return (claudecodeproto.Ensured{Installed: installed}).MarshalJSON()
	case claudecodeproto.OperationStatus:
		result, err := status(ctx, artifacts)
		if err != nil {
			return nil, err
		}
		return (claudecodeproto.StatusDone{Status: result}).MarshalJSON()
	}
	return nil, fmt.Errorf("unknown operation: %s", operation)
}

func readInput(ctx context.Context, input *commandsdk.Input) ([]byte, error) {
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
			return nil, &operationError{claudecodeproto.InvalidRelease, err}
		}
		if len(chunk) > maxInputBytes-len(body) {
			return nil, &operationError{
				claudecodeproto.InvalidRelease,
				fmt.Errorf("input exceeds %d bytes", maxInputBytes),
			}
		}
		body = append(body, chunk...)
	}
}
