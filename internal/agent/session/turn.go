package session

import (
	"context"
	"fmt"
	"math/rand"
	"slices"
	"time"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

func (s *Session) runTurn(ctx context.Context, switchFirst bool) error {
	compactions := 0
	continues := false
	for {
		if err := ctx.Err(); err != nil {
			return err
		}
		if switchFirst {
			compacted, err := s.applySwitch(ctx)
			if err != nil {
				return err
			}
			if compacted {
				s.pushResume()
			}
		}
		switchFirst = true
		if err := s.writeInputs(ctx); err != nil {
			return err
		}
		s.mu.Lock()
		before := s.core.arrivals
		s.mu.Unlock()
		needsCompaction, err := s.stream(ctx, continues)
		if err != nil {
			return err
		}
		continues = true
		if !needsCompaction {
			if err = s.writeInputsSince(ctx, before); err != nil {
				return err
			}
		}
		executed, stop, err := s.runTools(ctx, needsCompaction)
		if err != nil {
			return err
		}
		if needsCompaction && compactions < 3 {
			beforeSize, err := s.estimate(ctx)
			if err != nil {
				return err
			}
			compacted, err := s.compactPass(ctx)
			if err != nil {
				return err
			}
			afterSize, err := s.estimate(ctx)
			if err != nil {
				return err
			}
			if compacted && afterSize < beforeSize {
				compactions++
				s.pushResume()
				continue
			}
		}
		s.mu.Lock()
		arrivals := s.core.arrivals
		s.mu.Unlock()
		if arrivals > before {
			if err = s.writeInputs(ctx); err != nil {
				return err
			}
			continue
		}
		if stop || !executed {
			return nil
		}
	}
}

func (s *Session) stream(ctx context.Context, continues bool) (bool, error) {
	s.mutate(func(c *coreState) { c.stage = ProviderStreaming })
	attempt := uint32(1)
	sizeChecked, refusalCompacted := false, false
	for {
		request, err := s.request(ctx)
		if err != nil {
			return false, err
		}
		if !sizeChecked {
			sizeChecked = true
			s.mu.Lock()
			runtime, model := s.core.provider, s.core.model
			s.mu.Unlock()
			if s.deps.Config.Compaction.sizeReached(runtime.RequestLimits(model.Model), transcript.MeasureRequest(request.SystemPrompt, request.Items)) {
				did, err := s.compactPass(ctx)
				if err != nil {
					return false, err
				}
				if did {
					if continues {
						s.pushResume()
					}
					request, err = s.request(ctx)
					if err != nil {
						return false, err
					}
				}
			}
		}
		start := len(s.Transcript().Blocks)
		runtime, err := s.useProvider(ctx)
		if err != nil {
			return false, err
		}
		needsCompaction, failure, err := s.read(ctx, runtime.Run(ctx, request))
		s.mutate(func(c *coreState) { c.providerBusy = false })
		if err != nil {
			return false, err
		}
		if failure == nil {
			s.mutate(func(c *coreState) { c.stage = preparing })
			return needsCompaction, nil
		}
		if failure.Diagnostics == nil {
			failure.Diagnostics = &core.ProviderErrorDiagnostics{Source: "unknown"}
		} else {
			failure.Diagnostics = new(*failure.Diagnostics)
		}
		failure.Diagnostics.ClientRequestID = new(request.RequestID)
		unwindable := transcript.Cut(s.Transcript().Blocks).Cut <= start
		tooLarge := failure.Code != nil && *failure.Code == provider.ContextLengthExceeded
		if tooLarge && unwindable && !refusalCompacted && s.deps.Config.Compaction.ThresholdPercent != nil {
			refusalCompacted = true
			if err = s.restoreCommands(ctx, start); err != nil {
				return false, err
			}
			did, err := s.compactPass(ctx)
			if err != nil {
				return false, err
			}
			if did {
				if continues {
					s.pushResume()
				}
				continue
			}
		}
		policy := s.deps.Config.Retry
		if !unwindable || !policy.retries(attempt, *failure) {
			report := failureReport(*failure)
			s.mutate(func(c *coreState) { c.log.PushError(c.model, report.Message, report.Code, report.Diagnostics) })
			return false, report
		}
		if err = s.restoreCommands(ctx, start); err != nil {
			return false, err
		}
		delay := policy.delay(attempt, failure.RetryAfter)
		report := failureReport(*failure)
		s.emit(&RetryScheduled{Attempt: attempt, DelayMS: uint64(delay.Milliseconds()), Code: report.Code, Diagnostics: report.Diagnostics})
		timer := time.NewTimer(delay)
		select {
		case <-timer.C:
		case <-ctx.Done():
			timer.Stop()
			return false, ctx.Err()
		}
		timer.Stop()
		attempt++
	}
}

func failureReport(f provider.Failure) *ErrorReport {
	report := &ErrorReport{Message: f.Message, Diagnostics: f.Diagnostics}
	if f.Code != nil {
		report.Code = new(string(*f.Code))
	}
	return report
}
func (p RetryPolicy) retries(attempt uint32, f provider.Failure) bool {
	return f.Code != nil && (*f.Code == provider.RateLimit || *f.Code == provider.Overloaded) && attempt < p.MaxAttempts && (f.RetryAfter == nil || *f.RetryAfter <= p.MaxDelay)
}
func (p RetryPolicy) delay(attempt uint32, vendor *time.Duration) time.Duration {
	if vendor != nil {
		return min(*vendor, p.MaxDelay)
	}
	ceiling := p.BaseDelay
	for i := uint32(1); i < attempt && ceiling < p.MaxDelay; i++ {
		if ceiling > p.MaxDelay/2 {
			ceiling = p.MaxDelay
		} else {
			ceiling *= 2
		}
	}
	ceiling = min(ceiling, p.MaxDelay)
	ms := ceiling.Milliseconds()
	if ms <= 0 {
		return 0
	}
	return time.Duration(rand.Int63n(ms)) * time.Millisecond
}

func (s *Session) read(ctx context.Context, events provider.Run) (bool, *provider.Failure, error) {
	thinkingStarted, needsCompaction := false, false
	for event := range events {
		if err := ctx.Err(); err != nil {
			return false, nil, err
		}
		if _, ok := event.(*provider.ThinkingStart); ok {
			thinkingStarted = true
			continue
		}
		if failure, ok := event.(*provider.Error); ok {
			return false, new(failure.Failure), nil
		}
		_, text := event.(*provider.TextDelta)
		_, signature := event.(*provider.ThinkingSignature)
		if thinkingStarted || (!text && !signature) {
			if err := s.completeText(ctx); err != nil {
				return false, nil, err
			}
		}
		s.mutate(func(c *coreState) {
			if thinkingStarted {
				if delta, ok := event.(*provider.ThinkingDelta); ok {
					c.log.OpenThinking(c.model, delta.Text)
					return
				}
				c.log.OpenThinking(c.model, "")
			}
			switch e := event.(type) {
			case *provider.ThinkingStart:
			case *provider.ThinkingDelta:
				c.log.AppendThinking(c.model, e.Text)
			case *provider.ThinkingSignature:
				c.log.SignThinking(e.Signature)
			case *provider.RedactedThinking:
				c.log.PushRedactedThinking(c.model, e.Data)
			case *provider.TextDelta:
				c.log.AppendText(c.model, e.Text)
			case *provider.ToolCall:
				c.log.PushToolCall(c.model, *e)
			case *provider.Response:
				c.log.PushResponse(c.model, e.Usage)
				needsCompaction = needsCompaction || s.deps.Config.Compaction.reached(c.model.Model.ContextWindow, e.Usage)
			case *provider.Error:
			}
		})
		thinkingStarted = false
	}
	if err := ctx.Err(); err != nil {
		return false, nil, err
	}
	if err := s.completeText(ctx); err != nil {
		return false, nil, err
	}
	return needsCompaction, nil, nil
}
func (s *Session) completeText(ctx context.Context) error {
	s.mu.Lock()
	open := s.core.log.EndsWithOpenText()
	s.mu.Unlock()
	if !open {
		return nil
	}
	permit, err := s.persist.Acquire(ctx)
	if err != nil {
		return err
	}
	defer permit.Release()
	s.mutate(func(c *coreState) {
		if id := c.log.CompleteTailText(); id != nil {
			c.commands.Capture(*id, store.AfterAssistant, c.commands.Revision())
		}
	})
	return nil
}

func (s *Session) runTools(ctx context.Context, deferInput bool) (bool, bool, error) {
	s.mu.Lock()
	calls := s.core.log.PendingToolCalls()
	s.mu.Unlock()
	if len(calls) == 0 {
		return false, false, nil
	}
	s.mutate(func(c *coreState) { c.stage = ToolExecuting })
	if err := s.Flush(ctx); err != nil {
		return false, false, err
	}
	tools := s.deps.Runtime.Tools()
	stop := false
	for _, call := range calls {
		if err := ctx.Err(); err != nil {
			return true, stop, err
		}
		s.mu.Lock()
		before := s.core.arrivals
		s.mu.Unlock()
		var outcome ToolOutcome
		if slices.ContainsFunc(tools, func(t provider.ToolDefinition) bool { return t.Name == call.ToolName }) {
			s.mu.Lock()
			model, runtime, generation := s.core.model, s.core.provider, s.core.generation
			s.mu.Unlock()
			invocation := ToolInvocation{ToolUseID: call.ToolUseID, ToolName: call.ToolName, Input: transcript.ToolInput(call.Input), Model: model, RequestLimits: runtime.RequestLimits(model.Model), Generation: generation}
			var err error
			outcome, err = s.deps.Runtime.InvokeTool(ctx, invocation)
			if ctx.Err() != nil {
				return true, stop, ctx.Err()
			}
			if err != nil {
				outcome = ErrorOutcome("Tool failed: " + err.Error())
			}
		} else {
			outcome = ErrorOutcome("Tool not found: " + call.ToolName)
		}
		switch effect := outcome.Effect.(type) {
		case *ScheduleYield:
			stop = true
			var id core.WakeupID
			s.mutate(func(c *coreState) {
				id = core.WakeupID(s.deps.IDs.NextID())
				c.wakeups = append(c.wakeups, store.ScheduledWakeup{ID: id, DurationMS: effect.DurationMS})
				c.dirty = true
			})
			outcome = ToolOutcome{Output: []provider.ResultPart{&provider.TextPart{Text: fmt.Sprintf("yield scheduled\ndurationMs: %d", effect.DurationMS)}}, View: &core.YieldWakeup{WakeupID: id, DurationMs: effect.DurationMS}}
		}
		output, held := store.PersistResult(context.WithoutCancel(ctx), outcome.Output, s.deps.Store.Blobs())
		s.mutate(func(c *coreState) {
			c.media.Absorb(held)
			c.log.CompleteToolCall(call.ToolUseID, output, outcome.IsError, outcome.View)
		})
		if !deferInput {
			if err := s.writeInputsSince(ctx, before); err != nil {
				return true, stop, err
			}
		}
	}
	s.mutate(func(c *coreState) { c.stage = preparing })
	return true, stop, nil
}
