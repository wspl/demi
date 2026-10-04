package claudecode

import (
	"context"
	"errors"
	"io"
	"log/slog"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

// runtime is used serially by its session, as provider.Runtime requires.
type runtime struct {
	owner     *Provider
	placement Placement
	live      *live
}

// Fresh returns an independent runtime for another session.
func (r *runtime) Fresh() provider.Runtime { return &runtime{owner: r.owner, placement: r.placement} }

// RequestLimits returns the request limits for the model.
func (*runtime) RequestLimits(model types.Model) provider.RequestLimits {
	return provider.AnthropicRequestLimits(model)
}

// Close releases the runtime resources.
func (r *runtime) Close(ctx context.Context) error {
	if r.live == nil {
		return nil
	}
	l := r.live
	r.live = nil
	return l.close(ctx, true)
}

// Run streams inference events for the request.
func (r *runtime) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	return func(yield func(provider.Event) bool) {
		slog.DebugContext(
			ctx,
			"Claude Code run",
			"provider",
			r.owner.config.ID,
			"session",
			request.SessionID,
			"request",
			request.RequestID,
		)
		var current *live
		graceful := true
		defer func() {
			if current != nil {
				if err := current.close(ctx, graceful); err != nil {
					slog.DebugContext(ctx, "Claude Code process cleanup", "error", err)
				}
			}
		}()
		fail := func(err error) {
			if ctx.Err() != nil {
				return
			}
			failure := provider.Failure{Message: err.Error()}
			var f *provider.Failure
			if errors.As(err, &f) {
				failure = *f
			}
			if !yield(&provider.Error{Failure: failure}) {
				graceful = false
			}
		}
		current = r.live
		r.live = nil
		if err := r.prepareRun(ctx, request, &current); err != nil {
			fail(err)
			return
		}
		if current == nil {
			return
		}
		emit := func(events []provider.Event) bool {
			return emitRunEvents(ctx, events, &graceful, yield)
		}
		keep := func() {
			r.live = current
			current = nil
		}
		readRun(ctx, current, &graceful, fail, emit, keep, yield)
	}
}

// prepareRun leaves current nil when cancellation prevents starting a process.
func (r *runtime) prepareRun(ctx context.Context, request provider.InferenceRequest, current **live) error {
	if *current != nil && (*current).serves(request) {
		return (*current).continueRun(ctx, request)
	}
	if *current != nil {
		if err := (*current).close(ctx, true); err != nil {
			slog.DebugContext(ctx, "Claude Code process cleanup", "error", err)
		}
		*current = nil
	}
	if ctx.Err() != nil {
		return nil
	}
	var err error
	*current, err = startLive(ctx, r.owner, r.placement, request)
	if err != nil {
		return err
	}
	if err = (*current).prepare(ctx, request); err != nil {
		return err
	}
	return nil
}

func readRun(
	ctx context.Context,
	current *live,
	graceful *bool,
	fail func(error),
	emit func([]provider.Event) bool,
	keep func(),
	yield func(provider.Event) bool,
) {
	for {
		if err := current.flushReplies(ctx); err != nil {
			fail(err)
			return
		}
		queued, err := current.next(ctx)
		if errors.Is(err, io.EOF) {
			if err := current.finish(ctx); err != nil {
				fail(err)
			}
			*graceful = false
			return
		}
		if err != nil {
			fail(err)
			return
		}
		line, ok, err := current.read(queued)
		if err != nil {
			fail(err)
			return
		}
		if !ok {
			continue
		}
		switch line.Type {
		case "assistant":
			if current.assistant(&line, queued, fail, emit) {
				return
			}
		case "stream_event":
			if current.streamEvent(&line, queued, fail, emit, keep, yield) {
				return
			}
		case "control_request":
			if current.controlEvent(ctx, &line, fail, keep, yield) {
				return
			}
		case "result":
			current.collecting = nil
			keep()
			yield(line.Result.end(queued.text))
			return
		case "error":
			outputFailure(&line, queued, fail)
			return
		case "control_response":
		}
	}
}

// assistant returns true when the line ends the run.
func (l *live) assistant(line *outputLine, q queuedLine, fail func(error), emit func([]provider.Event) bool) bool {
	m := line.Assistant
	if m.Error.Value != nil || m.Message == nil || m.Message.Content == nil {
		return false
	}
	for _, block := range *m.Message.Content {
		if block.Type == "tool_use" {
			call, err := block.call()
			if err != nil {
				f := provider.ProtocolFailure(err.Error(), q.text)
				fail(&f)
				return true
			}
			l.collect(call)
		} else if !l.streamed && !emit(block.events()) {
			return true
		}
	}
	return false
}

// streamEvent returns true after completion or transferring the process back to its runtime.
func (l *live) streamEvent(
	line *outputLine,
	q queuedLine,
	fail func(error),
	emit func([]provider.Event) bool,
	keep func(),
	yield func(provider.Event) bool,
) bool {
	l.streamed = true
	event := line.Stream.Event
	if event == nil {
		return false
	}
	if event.Type == "message_stop" && len(l.collecting) > 0 {
		if l.mcp == nil {
			f := provider.ProtocolFailure("Claude Code called a tool, but the request offered none", q.text)
			fail(&f)
			return true
		}
		l.held = l.collecting
		l.collecting = nil
		batch := l.held
		keep()
		for _, call := range batch {
			if !yield(call) {
				return true
			}
		}
		return true
	}
	if !emit(event.events()) {
		return true
	}
	return false
}

func outputFailure(line *outputLine, q queuedLine, fail func(error)) {
	message := "Claude Code error"
	if line.Error.Message.Value != nil {
		message = *line.Error.Message.Value
	}
	failure := provider.ProtocolFailure(message, q.text)
	failure.Code = provider.ClassifyError(line.Error.Code.Value, message)
	fail(&failure)
}

// controlEvent returns true when a pending tool call transfers the process back to its runtime.
func (l *live) controlEvent(
	ctx context.Context,
	line *outputLine,
	fail func(error),
	keep func(),
	yield func(provider.Event) bool,
) bool {
	call, err := l.control(ctx, *line.Control)
	if err != nil {
		fail(err)
		return true
	}
	if call != nil && !l.streamed {
		l.collecting = nil
		l.held = []*provider.ToolCall{call}
		keep()
		yield(call)
		return true
	}
	return false
}

func emitRunEvents(ctx context.Context, events []provider.Event, graceful *bool, yield func(provider.Event) bool) bool {
	for _, event := range events {
		if ctx.Err() != nil {
			return false
		}
		if !yield(event) {
			*graceful = false
			return false
		}
	}
	return true
}
