package claudecode

import (
	"context"
	"errors"
	"io"
	"log/slog"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// runtime is used serially by its session, as provider.Runtime requires.
type runtime struct {
	owner     *Provider
	placement Placement
	live      *live
}

func (r *runtime) Fresh() provider.Runtime { return &runtime{owner: r.owner, placement: r.placement} }
func (*runtime) RequestLimits(model core.Model) provider.RequestLimits {
	return provider.AnthropicRequestLimits(model)
}
func (r *runtime) Close(ctx context.Context) error {
	if r.live == nil {
		return nil
	}
	l := r.live
	r.live = nil
	return l.close(ctx, true)
}
func (r *runtime) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	return func(yield func(provider.Event) bool) {
		slog.DebugContext(ctx, "Claude Code run", "provider", r.owner.config.ID, "session", request.SessionID, "request", request.RequestID)
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
		if current != nil && current.serves(request) {
			if err := current.continueRun(ctx, request); err != nil {
				fail(err)
				return
			}
		} else {
			if current != nil {
				if err := current.close(ctx, true); err != nil {
					slog.DebugContext(ctx, "Claude Code process cleanup", "error", err)
				}
				current = nil
			}
			if ctx.Err() != nil {
				return
			}
			var err error
			current, err = startLive(ctx, r.owner, r.placement, request)
			if err != nil {
				fail(err)
				return
			}
			if err = current.prepare(ctx, request); err != nil {
				fail(err)
				return
			}
		}
		emit := func(events []provider.Event) bool {
			for _, event := range events {
				if ctx.Err() != nil {
					return false
				}
				if !yield(event) {
					graceful = false
					return false
				}
			}
			return true
		}
		keep := func() {
			r.live = current
			current = nil
		}
		for {
			if err := current.flushReplies(ctx); err != nil {
				fail(err)
				return
			}
			q, err := current.next(ctx)
			if errors.Is(err, io.EOF) {
				if err := current.finish(ctx); err != nil {
					fail(err)
				}
				graceful = false
				return
			}
			if err != nil {
				fail(err)
				return
			}
			line, err := current.read(q)
			if err != nil {
				fail(err)
				return
			}
			if line == nil {
				continue
			}
			switch line.Type {
			case "assistant":
				m := line.Assistant
				if m.Error.Value != nil || m.Message == nil || m.Message.Content == nil {
					continue
				}
				for _, block := range *m.Message.Content {
					if block.Type == "tool_use" {
						call, err := block.call()
						if err != nil {
							f := provider.ProtocolFailure(err.Error(), q.text)
							fail(&f)
							return
						}
						current.collect(call)
					} else if !current.streamed && !emit(block.events()) {
						return
					}
				}
			case "stream_event":
				current.streamed = true
				event := line.Stream.Event
				if event == nil {
					continue
				}
				if event.Type == "message_stop" && len(current.collecting) > 0 {
					if current.mcp == nil {
						f := provider.ProtocolFailure("Claude Code called a tool, but the request offered none", q.text)
						fail(&f)
						return
					}
					current.held = current.collecting
					current.collecting = nil
					batch := current.held
					keep()
					for _, call := range batch {
						if !yield(call) {
							return
						}
					}
					return
				}
				if !emit(event.events()) {
					return
				}
			case "control_request":
				call, err := current.control(ctx, *line.Control)
				if err != nil {
					fail(err)
					return
				}
				if call != nil && !current.streamed {
					current.collecting = nil
					current.held = []*provider.ToolCall{call}
					keep()
					yield(call)
					return
				}
			case "result":
				current.collecting = nil
				keep()
				yield(line.Result.end(q.text))
				return
			case "error":
				message := "Claude Code error"
				if line.Error.Message.Value != nil {
					message = *line.Error.Message.Value
				}
				failure := provider.ProtocolFailure(message, q.text)
				failure.Code = provider.ClassifyError(line.Error.Code.Value, message)
				fail(&failure)
				return
			case "control_response":
			}
		}
	}
}
