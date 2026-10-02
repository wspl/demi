package jobs

import (
	"context"
	"errors"
	"io"
	"time"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerwire"
)

// send publishes a runner frame while the call and connection remain alive.
func (h *ConnectionHandle) send(ctx context.Context, message runnerwire.Outbound) error {
	frame, err := runnerwire.Encode(message)
	if err != nil {
		return err
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-h.Done():
		return context.Canceled
	case h.Control <- frame:
		return nil
	}
}

// retire sends cleanup without allowing a stalled backend to retain RPC work.
func (h *ConnectionHandle) retire(message runnerwire.Outbound) {
	frame, err := runnerwire.Encode(message)
	if err == nil {
		select {
		case h.Control <- frame:
			return
		default:
		}
	}
	h.cancel()
}

// runningHint pairs a displayed invocation hint with its unconditional clearing.
func runningHint(ctx context.Context, h *ConnectionHandle, job string, hint *string) (func(), error) {
	if hint == nil {
		return func() {}, nil
	}
	id := executionID()
	clearHint := func() { h.retire(&runnerwire.JobRunningHint{JobID: job, InvocationID: id}) }
	if err := h.send(ctx, &runnerwire.JobRunningHint{JobID: job, InvocationID: id, Hint: hint}); err != nil {
		clearHint()
		return nil, err
	}
	return clearHint, nil
}

// invokeRPC owns one callback's control routing and independently flowing pipes.
func invokeRPC(ctx context.Context, pipes *process.PipeClient, e *ExecutionContext, raw process.RawCommand, parsed *declare.Parsed, inv cmdsdk.InvocationContext[commandwire.LocalInvocation], output *commandOutput, finite bool) (code uint8, err error) {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	connection := e.Connection
	id := executionID()
	events := make(chan CallEvent, 8)
	if err = connection.RegisterCall(ctx, id, events, ctx.Done(), cancel); err != nil {
		return 0, err
	}
	defer func() {
		if err != nil {
			connection.retire(&runnerwire.RPCCancel{CallID: id})
		}
	}()
	args, err := parsed.Values.MarshalJSON()
	if err != nil {
		return 0, err
	}
	argv := append([]string{raw.Root}, raw.Argv...)
	if err = connection.send(ctx, &runnerwire.RPCCall{JobID: e.JobID, CallID: id, Root: raw.Root, Path: parsed.Path, Argv: argv, Args: args, JSON: parsed.JSON, CWD: inv.Request.Cwd, Env: inv.Request.Env, Stdin: finite}); err != nil {
		return 0, err
	}
	if !raw.Live {
		if err = connection.send(ctx, &runnerwire.RPCStdinEnd{CallID: id}); err != nil {
			return 0, err
		}
	}
	pulls := make(chan struct{}, 1)
	inputDone := make(chan error, 1)
	outputDone := make(chan error, 1)
	inputRef := make(chan *runnerwire.PipeRef, 1)
	outputRef := make(chan *runnerwire.PipeRef, 1)
	go func() {
		var reference *runnerwire.PipeRef
		select {
		case <-ctx.Done():
			inputDone <- ctx.Err()
			return
		case reference = <-inputRef:
		}
		inputDone <- rpcInput(ctx, pipes, connection, id, raw.Live, reference, pulls, inv.Input)
	}()
	go func() {
		var reference *runnerwire.PipeRef
		select {
		case <-ctx.Done():
			outputDone <- ctx.Err()
			return
		case reference = <-outputRef:
		}
		if reference == nil {
			outputDone <- nil
			return
		}
		result := rpcDownload(ctx, pipes, reference, output)
		reportErr := process.ReportPipe(ctx, connection.Control, reference.ID, result)
		outputDone <- errors.Join(result, reportErr)
	}()
	var inputResult, outputResult error
	inputFinished, outputFinished := false, false
	defer func() {
		cancel()
		if !inputFinished {
			<-inputDone
		}
		if !outputFinished {
			<-outputDone
		}
	}()
	var inputTimeout *time.Timer
	var deadline <-chan time.Time
	defer func() {
		if inputTimeout != nil {
			inputTimeout.Stop()
		}
	}()
	receivedPipes, exited := false, false
	for {
		if exited && outputFinished {
			return code, outputResult
		}
		select {
		case <-ctx.Done():
			return 0, ctx.Err()
		case <-connection.Done():
			return 0, context.Canceled
		case <-deadline:
			return 0, inputResult
		case result := <-inputDone:
			inputFinished = true
			inputDone = nil
			inputResult = result
			if result != nil {
				inputTimeout = time.NewTimer(15 * time.Second)
				deadline = inputTimeout.C
			}
		case result := <-outputDone:
			outputFinished = true
			outputDone = nil
			outputResult = result
			if result != nil {
				return 0, result
			}
		case event := <-events:
			switch event := event.(type) {
			case *CallPipes:
				if receivedPipes {
					return 0, errors.New("duplicate RPC pipes")
				}
				if (event.Stdin != nil) != finite {
					return 0, errors.New("RPC input pipe disagrees with invocation")
				}
				receivedPipes = true
				inputRef <- event.Stdin
				outputRef <- &event.Stdout
			case *CallStderr:
				if err := output.Stderr(ctx, event.Bytes); err != nil {
					return 0, err
				}
			case *CallPull:
				if raw.Live {
					select {
					case pulls <- struct{}{}:
					default:
						return 0, errors.New("overlapping RPC stdin demands")
					}
				} else {
					if err := connection.send(ctx, &runnerwire.RPCStdinEnd{CallID: id}); err != nil {
						return 0, err
					}
				}
			case *CallExit:
				if !receivedPipes {
					if event.ExitCode == 0 {
						return 0, errors.New("RPC success arrived before pipe descriptors")
					}
					inputRef <- nil
					outputRef <- nil
					receivedPipes = true
				}
				code = event.ExitCode
				exited = true
			}
		}
	}
}

// rpcInput forwards only demanded live input, or a finite HTTP upload.
func rpcInput(ctx context.Context, pipes *process.PipeClient, h *ConnectionHandle, id string, live bool, reference *runnerwire.PipeRef, pulls <-chan struct{}, input *cmdsdk.Input) error {
	if live {
		for {
			select {
			case <-ctx.Done():
				return ctx.Err()
			case <-pulls:
			}
			bytes, err := input.Next(ctx)
			if errors.Is(err, io.EOF) {
				return h.send(ctx, &runnerwire.RPCStdinEnd{CallID: id})
			}
			if err != nil {
				return err
			}
			if err = h.send(ctx, &runnerwire.RPCStdin{CallID: id, Bytes: bytes}); err != nil {
				return err
			}
		}
	}
	if reference == nil {
		return nil
	}
	body := newInvocationBody(ctx, input)
	result := pipes.Put(ctx, reference.URL, body)
	return errors.Join(result, process.ReportPipe(ctx, h.Control, reference.ID, result))
}

// invocationBody adapts demand-driven command chunks to a pipe upload without read-ahead.
type invocationBody struct {
	ctx     context.Context
	input   *cmdsdk.Input
	pending []byte
	cancel  context.CancelFunc
}

func (b *invocationBody) Read(bytes []byte) (int, error) {
	for len(b.pending) == 0 {
		chunk, err := b.input.Next(b.ctx)
		if err != nil {
			return 0, err
		}
		b.pending = chunk
	}
	n := copy(bytes, b.pending)
	b.pending = b.pending[n:]
	return n, nil
}
func (b *invocationBody) Close() error { b.cancel(); return nil }

// newInvocationBody gives each pipe upload an independently cancellable input read.
func newInvocationBody(ctx context.Context, input *cmdsdk.Input) *invocationBody {
	lifetime, cancel := context.WithCancel(ctx)
	return &invocationBody{ctx: lifetime, input: input, cancel: cancel}
}

// rpcDownload preserves callback stdout until its pipe has drained completely.
func rpcDownload(ctx context.Context, pipes *process.PipeClient, reference *runnerwire.PipeRef, output *commandOutput) error {
	body, err := pipes.Open(ctx, reference.URL)
	if err != nil {
		return err
	}
	defer func() { _ = body.Close() }() // Cleanup follows the operation result; cancellation may already have closed it.
	buffer := make([]byte, 65536)
	for {
		n, err := body.Read(buffer)
		if n > 0 {
			if err := output.Stdout(ctx, buffer[:n]); err != nil {
				return err
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
