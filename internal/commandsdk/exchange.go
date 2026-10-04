package commandsdk

import (
	"context"
	"errors"
	"fmt"
	"io"

	"github.com/wspl/demi/internal/commandproto"
)

// ExchangeError identifies the failing side while preserving its cause.
type ExchangeError struct {
	Side  string
	Cause error
}

// Error returns the failure message.
func (e *ExchangeError) Error() string { return fmt.Sprintf("command %s: %v", e.Side, e.Cause) }

// Unwrap returns the underlying failure.
func (e *ExchangeError) Unwrap() error { return e.Cause }

// Exchange drives one invocation's pull-driven input and output to completion.
type Exchange struct {
	Input  *CommandInput
	Output *CommandOutput
}

// Run forwards input only on demand and cancels outstanding work on any failure.
func (e Exchange) Run(ctx context.Context, source InputSource, sink OutputSink) (commandproto.Completion, error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	pulls := make(chan struct{}, 1)
	sent := make(chan struct{})
	var inputErr error
	go func() {
		inputErr = e.forwardInput(ctx, source, pulls)
		close(sent)
		if inputErr != nil {
			cancel()
			e.Input.Cancel()
		}
	}()
	defer func() {
		cancel()
		e.Input.Cancel()
		<-sent
	}()
	var completion commandproto.Completion
	for {
		r, err := e.Output.Next(ctx)
		if err != nil {
			select {
			case <-sent:
				if inputErr != nil {
					return completion, inputErr
				}
			default:
			}
			if errors.Is(err, io.EOF) {
				return completion, nil
			}
			return completion, &ExchangeError{Side: "service", Cause: err}
		}
		switch r := r.(type) {
		case commandproto.Stdout:
			err = sink.Stdout(ctx, r)
		case commandproto.Stderr:
			err = sink.Stderr(ctx, r)
		case commandproto.Completed:
			completion = r.Completion
		case commandproto.InputPull:
			select {
			case pulls <- struct{}{}:
			default:
				return completion, &ExchangeError{Side: "service", Cause: errors.New("overlapping input demands")}
			}
		}
		if err != nil {
			return completion, &ExchangeError{Side: "output", Cause: err}
		}
	}
}

// forwardInput sends one input chunk for each service demand.
func (e Exchange) forwardInput(ctx context.Context, source InputSource, pulls <-chan struct{}) error {
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-pulls:
		}
		b, err := source.Next(ctx)
		if errors.Is(err, io.EOF) {
			return e.Input.End()
		}
		if err != nil {
			return &ExchangeError{Side: "input", Cause: err}
		}
		if err = e.Input.Write(ctx, b); err != nil {
			return &ExchangeError{Side: "service", Cause: err}
		}
	}
}
