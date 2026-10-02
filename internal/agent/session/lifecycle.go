package session

import (
	"context"
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
)

// construct installs one session state owner before starting its three workers.
func construct(init Init, deps Deps, log *transcript.Log, commands *store.CommandStateHistory, state *store.CheckpointState) *Session {
	ctx, cancel := context.WithCancel(context.Background())
	generation, generationCancel := context.WithCancel(context.Background())
	c := coreState{id: init.ID, cwd: init.CWD, model: init.Model, log: log, commands: commands, provider: init.Runtime, stage: Idle, changed: make(chan struct{}), generationCtx: generation, generationCancel: generationCancel, listeners: map[uint64]func(Event){}, edits: []store.EditReceipt{}, wakeups: []store.ScheduledWakeup{}, publishedPhase: "idle", publishedQueue: []core.QueuedMessage{}, publishedSteers: []core.PendingSteer{}}
	if state != nil {
		c.edits = append(c.edits, state.Edits...)
		c.wakeups = append(c.wakeups, state.Wakeups...)
		c.held = state.Phase != "idle"
		for _, input := range state.AgentInputs {
			c.inputs = append(c.inputs, pendingInput{agent: new(input)})
			c.arrivals++
		}
	}
	c.publishedStatus = c.status()
	c.publishedStatus.changed = make(chan struct{})
	s := &Session{sessionOwner: &sessionOwner{core: c, deps: deps, ctx: ctx, cancel: cancel, closed: make(chan struct{})}}
	s.mutate(func(_ *coreState) {
		s.armLocked()
	})
	s.workers.Go(s.worker)
	s.workers.Go(s.persister)
	s.workers.Go(s.wakeupDriver)
	return s
}

func restore(checkpoint store.Checkpoint, id core.NodeID, runtime provider.Runtime, deps Deps) (*Session, Continuation, error) {
	ids := map[core.BlockID]bool{}
	for _, block := range checkpoint.Transcript {
		ids[block.ID()] = true
	}
	for _, input := range checkpoint.State.AgentInputs {
		message := input.Message
		if message.RecipientID != id {
			return nil, Continuation{}, &RestoreError{Kind: RestoreInput, Detail: fmt.Sprintf("agent message %s is addressed to %s", message.ID, message.RecipientID)}
		}
		if ids[message.ID] {
			return nil, Continuation{}, &RestoreError{Kind: RestoreInput, Detail: fmt.Sprintf("agent message %s is not unique", message.ID)}
		}
		ids[message.ID] = true
	}
	wakeups := map[core.WakeupID]bool{}
	for _, w := range checkpoint.State.Wakeups {
		if wakeups[w.ID] {
			return nil, Continuation{}, &RestoreError{Kind: RestoreInput, Detail: fmt.Sprintf("wakeup %s is scheduled twice", w.ID)}
		}
		wakeups[w.ID] = true
	}
	edits := map[core.OperationID]bool{}
	for _, receipt := range checkpoint.State.Edits {
		if edits[receipt.OperationID] {
			return nil, Continuation{}, &RestoreError{Kind: RestoreEdits, Detail: string(receipt.OperationID)}
		}
		edits[receipt.OperationID] = true
	}
	commands, err := store.RestoreCommandStateHistory(checkpoint.CommandState)
	if err != nil {
		return nil, Continuation{}, &RestoreError{Kind: RestoreCommandState, Cause: err}
	}
	log := transcript.NewLog(checkpoint.Transcript, deps.IDs, deps.Clock)
	for _, call := range log.PendingToolCalls() {
		log.CompleteToolCall(call.ToolUseID, []core.ToolResultContentBlock{&core.ToolText{Text: fmt.Sprintf("Tool call interrupted: %s (the process died before a result was recorded)", call.ToolName)}}, true, nil)
	}
	init := Init{ID: id, CWD: checkpoint.State.CWD, Model: checkpoint.State.Model, Runtime: runtime}
	s := construct(init, deps, log, commands, &checkpoint.State)
	return s, Continuation{Interrupted: checkpoint.State.Phase != "idle", Queued: checkpoint.State.Queue}, nil
}

// dispose closes admission atomically, then joins workers and commits outside locks.
func (s *Session) dispose(ctx context.Context) error {
	first := false
	s.mutate(func(c *coreState) {
		if c.disposing {
			return
		}
		first = true
		c.disposing = true
		c.effects = append(c.effects, c.generationCancel, s.cancel)
		kept := []*action{}
		for _, a := range c.queue {
			c.effects = append(c.effects, func() { a.answer.finish(Detached, nil) })
			if a.kind == sendAction {
				kept = append(kept, a)
			}
		}
		c.queue = kept
		if c.active != nil && !c.active.stopped {
			c.active.shutdown = true
			c.effects = append(c.effects, c.active.cancel)
		}
	})
	if !first {
		<-s.closed
		return s.closeErr
	}
	cleanup := context.WithoutCancel(ctx)
	s.workers.Wait()
	saved := s.Flush(cleanup)
	// Wait for a public ForkRuntime call before closing its source.
	_, reserveErr := s.useProvider(cleanup)
	saved = errors.Join(saved, reserveErr)
	var runtimes []provider.Runtime
	s.mutate(func(c *coreState) {
		runtimes = append(runtimes, c.retired...)
		if c.provider != nil {
			runtimes = append(runtimes, c.provider)
		}
		if c.change != nil && c.change.Runtime != nil {
			runtimes = append(runtimes, c.change.Runtime)
		}
		if c.waitingChange != nil && c.waitingChange.Runtime != nil {
			runtimes = append(runtimes, c.waitingChange.Runtime)
		}
		c.retired = nil
		c.provider = nil
		c.change = nil
		c.waitingChange = nil
	})
	for _, runtime := range runtimes {
		saved = errors.Join(saved, runtime.Close(cleanup))
	}
	saved = errors.Join(saved, s.deps.Runtime.Dispose(cleanup))
	s.closeErr = saved
	close(s.closed)
	return saved
}

// stop records cancellation while ownership remains with the action worker.
func (s *Session) stop(ctx context.Context, onlyRunning bool) (framewire.AbortResult, error) {
	var a *action
	result := framewire.AbortResult{}
	s.mutate(func(c *coreState) {
		if c.disposing {
			return
		}
		if c.active != nil && c.stage != Finalizing && !c.active.stopped {
			a = c.active
			a.stopped = true
			target := framewire.AbortTargetActiveTurn
			switch c.stage {
			case ToolExecuting:
				target = framewire.AbortTargetActiveTool
			case Compacting:
				target = framewire.AbortTargetActiveCompaction
			case ProviderStreaming:
				target = framewire.AbortTargetActiveProviderStream
			}
			result.Target = &target
			c.effects = append(c.effects, a.cancel)
		} else if !onlyRunning {
			if len(c.queue) > 0 {
				dropped := c.queue[0]
				c.queue = c.queue[1:]
				target := framewire.AbortTargetQueuedAction
				if dropped.kind == sendAction {
					target = framewire.AbortTargetQueuedMessage
				}
				result.Target = &target
				c.effects = append(c.effects, func() { dropped.answer.finish(Dropped, nil) })
			} else if len(c.wakeups) > 0 {
				c.wakeups = c.wakeups[1:]
				c.dirty = true
				result.Target = new(framewire.AbortTargetPendingYieldWakeup)
			}
		}
		result.CanAbortAgain = c.canAbort()
	})
	if a != nil {
		select {
		case <-a.ack:
			result.CanAbortAgain = a.again
		case <-ctx.Done():
			return framewire.AbortResult{}, ctx.Err()
		}
	}
	return result, nil
}

func (s *Session) waitSettled(ctx context.Context) error {
	for {
		status := s.Status()
		if status.Settle != Busy {
			return nil
		}
		select {
		case <-status.Changed():
		case <-ctx.Done():
			return ctx.Err()
		}
	}
}
