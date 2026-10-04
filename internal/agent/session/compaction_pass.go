package session

import (
	"context"
	"errors"
	"strings"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

func (c CompactionConfig) tokenReached(window uint32, used uint64) bool {
	return c.ThresholdPercent != 0 && window > 0 && used >= uint64(window)*uint64(c.ThresholdPercent)/100
}

func (c CompactionConfig) reached(window uint32, usage types.TokenUsage) bool {
	return c.tokenReached(window, usage.InputTokens+usage.OutputTokens+usage.CacheReadTokens+usage.CacheWriteTokens)
}

func (c CompactionConfig) sizeReached(limits provider.RequestLimits, size transcript.RequestSize) bool {
	if c.ThresholdPercent == 0 {
		return false
	}
	percent := uint64(c.ThresholdPercent)
	return (limits.BodyBytes != nil && size.Bytes >= *limits.BodyBytes*percent/100) ||
		(limits.Images != nil && size.Images >= uint64(*limits.Images)*percent/100)
}

func (s *Session) preflight(ctx context.Context) error {
	estimate, err := s.estimate(ctx)
	if err != nil {
		return err
	}
	if s.deps.Config.Compaction.tokenReached(s.Model().Model.ContextWindow, estimate) {
		_, err = s.compactPass(ctx)
	}
	return err
}

func (s *Session) overThreshold(
	ctx context.Context,
	model types.ModelSelection,
	limits provider.RequestLimits,
) (bool, error) {
	view, err := s.modelView(ctx)
	if err != nil {
		return false, err
	}
	request := transcript.NewRequestView(view, model.Model, limits)
	if s.deps.Config.Compaction.tokenReached(model.Model.ContextWindow, transcript.Estimate(request)) {
		return true, nil
	}
	prompt, err := s.deps.Runtime.SystemPrompt(ctx)
	if err != nil {
		return false, err
	}
	replayed := transcript.Replay(request)
	return s.deps.Config.Compaction.sizeReached(limits, transcript.MeasureRequest(prompt, replayed.Items)), nil
}

func (s *Session) compactPass(ctx context.Context) (bool, error) {
	var stage Execution
	s.mutate(func(c *coreState) {
		stage = c.stage
		c.stage = Compacting
	})
	defer s.mutate(func(c *coreState) { c.stage = stage })
	view, err := s.modelView(ctx)
	if err != nil {
		return false, err
	}
	s.mu.Lock()
	pending := len(s.core.log.PendingToolCalls()) > 0
	model, runtime := s.core.model, s.core.provider
	s.mu.Unlock()
	if pending {
		return false, nil
	}
	window := transcript.Window(view.Blocks)
	first := firstCompactionBlock(view.Blocks, window.Start, window.Cut)
	if first < 0 {
		return false, nil
	}
	request := transcript.NewRequestView(view, model.Model, runtime.RequestLimits(model.Model))
	cut := window.Cut
	for {
		tokens := uint64(0)
		for _, b := range view.Blocks[window.Start:cut] {
			tokens += transcript.BlockTokens(b, request)
		}
		summary, err := s.summarize(ctx, view.Blocks[window.Start:cut])
		if err != nil {
			var report *ReportError
			if errors.As(err, &report) && report.Code != nil &&
				*report.Code == string(provider.ContextLengthExceeded) &&
				cut > first+1 {
				cut = max(window.Start+(cut-window.Start)/2, first+1)
				continue
			}
			return false, err
		}
		if summary == "" {
			return false, nil
		}
		s.mutate(func(c *coreState) {
			boundary := c.log.InsertCompactionBoundary(view.Start+cut, c.model, summary, transcript.TextTokens(summary))
			c.log.PushCompactionMarker(c.model, boundary, tokens)
			c.releaseMediaLocked()
		})
		if err = s.Flush(ctx); err != nil {
			return false, err
		}
		return true, nil
	}
}

func (s *Session) summarize(ctx context.Context, window []types.Block) (string, error) {
	runtime, err := s.forkRuntime(ctx)
	if err != nil {
		return "", err
	}
	s.mu.Lock()
	init := Init{ID: s.core.id, CWD: s.core.cwd, Model: s.core.model, Runtime: runtime}
	snapshot := s.core.commands.Select(window, s.core.commands.Revision(), false)
	held := s.core.media.Select(window)
	s.mu.Unlock()
	commands, err := store.RestoreCommandStateHistory(snapshot)
	if err != nil {
		return "", errors.Join(err, runtime.Close(context.WithoutCancel(ctx)))
	}
	deps := s.deps
	deps.Runtime = copyRuntime{Runtime: deps.Runtime}
	deps.Store = copyStore{}
	deps.Config.Compaction.ThresholdPercent = 0
	// Copies share the parent's identity and clock sources through its mutex.
	deps.IDs = sessionIDs{session: s}
	deps.Clock = sessionClock{session: s}
	log := transcript.NewLog(window, deps.IDs, deps.Clock)
	summarySession := construct(init, deps, log, commands, nil)
	summarySession.HoldMedia(&held)
	subscription := summarySession.Subscribe(func(event Event) {
		if retry, ok := event.(*RetryScheduled); ok {
			s.emit(retry)
		}
	})
	defer subscription.Release()
	answer, sendErr := summarySession.Send(
		[]types.UserContentBlock{&types.UserText{Text: CompactionSummaryInstruction}},
		types.TurnID(deps.IDs.NextID()),
	)
	var end ActionEnd
	if sendErr == nil {
		end, err = answer.Wait(ctx)
	} else {
		err = sendErr
	}
	if ctx.Err() != nil {
		_, stopErr := summarySession.Abort(context.WithoutCancel(ctx))
		err = errors.Join(err, stopErr)
	}
	text := ""
	if err == nil && end == Completed {
		text = strings.TrimSpace(transcript.LastAssistantText(summarySession.Transcript().Blocks, len(window)))
	}
	err = errors.Join(err, summarySession.Dispose(context.WithoutCancel(ctx)))
	return text, err
}

func firstCompactionBlock(blocks []types.Block, start, cut int) int {
	first := -1
	for i := start; i < cut; i++ {
		_, boundary := blocks[i].(*types.CompactionBoundaryBlock)
		_, marker := blocks[i].(*types.CompactionMarkerBlock)
		if !boundary && !marker {
			first = i
			break
		}
	}
	return first
}
