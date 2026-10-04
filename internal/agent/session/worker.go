package session

import (
	"context"
	"errors"
	"fmt"
	"slices"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/types"
)

// worker is the sole executor of session actions. Disposal cancels its action
// and joins this worker after the action has acknowledged its durable outcome.
func (s *Session) worker() {
	for {
		s.mu.Lock()
		a, changed, closing := s.core.active, s.core.changed, s.core.disposing
		s.mu.Unlock()
		if a == nil {
			if closing {
				return
			}
			select {
			case <-changed:
				continue
			case <-s.ctx.Done():
				continue
			}
		}
		s.runAction(a)
	}
}

func (s *Session) runAction(a *action) {
	lease, err := s.deps.Runtime.EnterAction(a.ctx)
	if lease != nil {
		defer lease.Release()
	}
	if err == nil {
		err = s.execute(a)
	}
	if err == nil && a.ctx.Err() != nil {
		err = a.ctx.Err()
	}
	cleanup := context.WithoutCancel(a.ctx)
	end, rejected, report := s.finishAction(a, err)
	s.mu.Lock()
	shutdown := s.core.disposing && s.core.active == nil
	s.mu.Unlock()
	if shutdown {
		return
	}
	if !rejected {
		if saved := s.Flush(cleanup); saved != nil {
			s.emit(&ErrorEvent{Report: ReportError{Message: saved.Error()}})
			if report == nil && end == Completed {
				report = &ReportError{Message: saved.Error()}
			}
		}
	}
	s.mutate(func(c *coreState) {
		c.active = nil
		c.stage = Idle
		s.startNextLocked()
		if report != nil {
			s.eventLocked(&ActionFailed{Report: *report})
			c.effects = append(c.effects, func() {
				a.answer.finish(0, report)
			})
		} else {
			c.effects = append(c.effects, func() {
				a.answer.finish(end, nil)
			})
		}
	})
}

func (s *Session) execute(a *action) error {
	ctx := a.ctx
	switch a.kind {
	case sendAction:
		if err := s.sendTurn(ctx, a); err != nil {
			return err
		}
	case continueAction:
		if _, err := s.applySwitch(ctx); err != nil {
			return err
		}
		opened, agents := s.continueInputs(a)
		if !opened {
			return nil
		}
		if agents {
			if err := s.Flush(ctx); err != nil {
				return err
			}
		}
	case retryAction:
		if err := s.retry(ctx); err != nil {
			return err
		}
	case resumeAction:
		if err := s.resumeTurn(ctx); err != nil {
			return err
		}
	case editAction:
		return s.runEdit(ctx)
	case compactAction:
		if _, err := s.compactPass(ctx); err != nil {
			return err
		}
		s.mu.Lock()
		agents := len(s.core.agentInputsLocked()) > 0
		s.mu.Unlock()
		if err := s.writeInputs(ctx); err != nil {
			return err
		}
		if !agents {
			return nil
		}
		return s.runTurn(ctx, true)
	}
	if err := s.preflight(ctx); err != nil {
		return err
	}
	return s.runTurn(ctx, true)
}

func (s *Session) retry(ctx context.Context) error {
	rewound, ok := transcript.Rewind(s.Transcript().Blocks)
	if !ok {
		return &ReportError{Message: "There is no input turn to retry"}
	}
	input := rewound.Retained[rewound.Input]
	edge := store.BeforeUser
	if _, ok := input.(*types.AgentMessageBlock); ok {
		edge = store.AfterBlock
	}
	s.mu.Lock()
	revision, ok := s.core.commands.Boundary(input.ID(), edge)
	s.mu.Unlock()
	if !ok {
		return &ReportError{Message: fmt.Sprintf("No command-state boundary for block %s", input.ID())}
	}
	if err := s.rewrite(ctx, rewound.Retained, revision); err != nil {
		return err
	}
	s.mutate(func(c *coreState) {
		c.active.turn = rewound.Turn
	})
	if err := ctx.Err(); err != nil {
		return err
	}
	_, err := s.applySwitch(ctx)
	return err
}

func (s *Session) restoreCommands(ctx context.Context, cut int) error {
	blocks := s.Transcript().Blocks[:cut]
	revision := uint64(0)
	if len(blocks) > 0 {
		last := blocks[len(blocks)-1]
		s.mu.Lock()
		var ok bool
		revision, ok = s.core.commands.Boundary(last.ID(), store.AfterBlock)
		s.mu.Unlock()
		if !ok {
			return &ReportError{Message: fmt.Sprintf("No command-state boundary after block %s", last.ID())}
		}
	}
	return s.rewrite(ctx, blocks, revision)
}

func (s *Session) abortCallsLocked() {
	c := &s.core
	for _, call := range c.log.PendingToolCalls() {
		c.log.CompleteToolCall(
			call.ToolUseID,
			[]types.ToolResultContentBlock{&types.ToolText{Text: "Tool call aborted: " + call.ToolName}},
			true,
			nil,
		)
	}
}

func (s *Session) recordStopLocked(shutdown bool) {
	c := &s.core
	take := exceptAgentMessages
	if shutdown {
		take = humanSteers
	}
	s.writeInputsLocked(take)
	s.abortCallsLocked()
	if shutdown {
		if !c.log.EndsWithInterruption() {
			c.log.PushError(c.model, transcript.InterruptedTurnMessage, new(transcript.InterruptedCode), nil)
		}
	} else {
		c.log.PushAbort(c.model)
	}
}

func (s *Session) pushResume() {
	s.mutate(func(c *coreState) {
		c.log.PushResume(c.active.turn, c.model)
	})
}

// finishAction records an action outcome before the final checkpoint is saved.
func (s *Session) finishAction(a *action, err error) (ActionEnd, bool, *ReportError) {
	end := Completed
	rejected := false
	var report *ReportError
	s.mutate(func(c *coreState) {
		if a.shutdown && errors.Is(err, context.Canceled) {
			end = s.detachActionLocked(a)
			rejected = true
			return
		}
		if c.preparingEditLocked() {
			editErr := stoppedEditError(err)
			s.rejectEditLocked(editErr)
			end = Dropped
			rejected = true
		} else if err != nil {
			end, report = s.actionErrorLocked(err)
		}
		a.stopped = true
		a.again = c.canAbortLocked()
		c.effects = append(c.effects, func() {
			close(a.ack)
		})
		c.stage = Finalizing
		c.edit = nil
		c.inputs = slices.DeleteFunc(c.inputs, func(input pendingInput) bool {
			return input.steer != nil
		})
		s.armLocked()
		c.releaseMediaLocked()
	})
	return end, rejected, report
}

// detachActionLocked preserves interrupted work for restoration while the session mutex is held.
func (s *Session) detachActionLocked(a *action) ActionEnd {
	c := &s.core
	end := Completed
	began := a.startRevision != c.log.Version().Revision
	s.rejectEditLocked(ErrClosed)
	if began {
		s.recordStopLocked(true)
		end = Aborted
	} else {
		end = Detached
		if a.kind == sendAction {
			c.queue = append([]*action{a}, c.queue...)
		}
	}
	s.armLocked()
	c.interrupted = began
	c.active = nil
	c.stage = Idle
	a.again = false
	c.effects = append(c.effects, func() {
		close(a.ack)
	}, func() {
		a.answer.finish(end, nil)
	}, a.cancel)
	return end
}

// stoppedEditError is the rejection of an edit whose action ended with err.
func stoppedEditError(err error) error {
	if err == nil || errors.Is(err, context.Canceled) {
		return ErrEditStopped
	}
	return err
}

func (s *Session) sendTurn(ctx context.Context, a *action) error {
	s.mu.Lock()
	revision := s.core.commands.Revision()
	s.mu.Unlock()
	if _, err := s.applySwitch(ctx); err != nil {
		return err
	}
	preamble, err := s.deps.Runtime.Preamble(ctx)
	if err != nil {
		return err
	}
	if err = ctx.Err(); err != nil {
		return err
	}
	s.mutate(func(c *coreState) {
		id := c.log.PushUser(a.turn, c.model, a.content, preamble)
		c.commands.Capture(id, store.BeforeUser, revision)
		a.content = nil
	})
	return nil
}

func (s *Session) resumeTurn(ctx context.Context) error {
	point := transcript.Cut(s.Transcript().Blocks)
	if point.FullRerun {
		return s.retry(ctx)
	}
	if err := s.restoreCommands(ctx, point.Cut); err != nil {
		return err
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	s.mutate(func(c *coreState) {
		c.log.MarkLatestAbortResumed()
	})
	if _, err := s.applySwitch(ctx); err != nil {
		return err
	}
	s.pushResume()
	return nil
}

func (s *Session) continueInputs(a *action) (bool, bool) {
	opened, agents := false, false
	s.mutate(func(c *coreState) {
		for i, input := range c.inputs {
			if input.wakeup != nil {
				c.log.PushWakeup(types.BlockID(input.wakeup.ID), a.turn, c.model, "new_turn")
				c.inputs = slices.Delete(c.inputs, i, i+1)
				c.dirty = true
				opened = true
				return
			}
		}
		for _, input := range c.inputs {
			agents = agents || input.agent != nil
		}
		if agents {
			opened = true
			s.writeInputsLocked(allInputs)
		}
	})
	return opened, agents
}

// actionErrorLocked records cancellation or failure while the session mutex is held.
func (s *Session) actionErrorLocked(err error) (ActionEnd, *ReportError) {
	end := Completed
	var report *ReportError
	if errors.Is(err, context.Canceled) {
		s.recordStopLocked(false)
		end = Aborted
	} else {
		if !errors.As(err, &report) {
			report = &ReportError{Message: err.Error()}
		}
		s.writeInputsLocked(allInputs)
		s.abortCallsLocked()
		s.eventLocked(&ErrorEvent{Report: *report})
	}
	return end, report
}
