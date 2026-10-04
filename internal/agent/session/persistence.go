package session

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
)

func (c *coreState) checkpointStateLocked() store.CheckpointState {
	phase := c.phaseLocked()
	if c.stage == Finalizing {
		phase = "idle"
	}
	return store.CheckpointState{
		Phase:       phase,
		Queue:       c.queuedMessagesLocked(),
		AgentInputs: c.agentInputsLocked(),
		Wakeups:     c.savedWakeupsLocked(),
		CWD:         c.cwd,
		Model:       c.model,
		Edits:       append([]store.EditReceipt{}, c.edits...),
	}
}

// save commits the session's dirty rows while the caller holds its save gate.
func (s *Session) save(ctx context.Context, pending *store.CommandVersion, guard store.CommitGuard) error {
	var update *store.CheckpointUpdate
	var rows transcript.DirtyRows
	s.mutate(func(c *coreState) {
		if !c.dirty && pending == nil {
			return
		}
		c.dirty = false
		rows = c.rows
		c.rows = transcript.DirtyRows{}
		blocks := c.log.Blocks()
		changed := []store.ChangedBlock{}
		for _, i := range rows.Indices(len(blocks)) {
			changed = append(changed, store.ChangedBlock{Index: i, Block: blocks[i]})
		}
		update = &store.CheckpointUpdate{
			State:         c.checkpointStateLocked(),
			CommandState:  c.commands.TakeUpdate(pending),
			ChangedBlocks: changed,
			BlockCount:    len(blocks),
		}
	})
	if update == nil {
		return nil
	}
	// Durable writes belong to the session, not the departing request.
	err := s.deps.Store.Save(context.WithoutCancel(ctx), *update, guard)
	if err != nil {
		s.mutate(func(c *coreState) {
			c.dirty = true
			c.rows.Merge(rows)
			if update.CommandState != nil {
				c.commands.MarkDirty()
			}
		})
	}
	return err
}

// persister owns the one-second throttle timer and waits for a new change after failure.
func (s *Session) persister() {
	for {
		s.mu.Lock()
		dirty, changed := s.core.dirty, s.core.changed
		s.mu.Unlock()
		if !dirty {
			select {
			case <-changed:
				continue
			case <-s.ctx.Done():
				return
			}
		}
		timer := time.NewTimer(s.deps.Config.PersistInterval)
		select {
		case <-timer.C:
		case <-s.ctx.Done():
			timer.Stop()
			return
		}
		timer.Stop()
		if err := s.Flush(s.ctx); err != nil {
			if s.ctx.Err() != nil {
				return
			}
			s.emit(&ErrorEvent{Report: ReportError{Message: err.Error()}})
			s.mu.Lock()
			changed = s.core.changed
			s.mu.Unlock()
			select {
			case <-changed:
			case <-s.ctx.Done():
				return
			}
		}
	}
}

// rewrite saves a candidate session history before publishing a replace patch.
func (s *Session) rewrite(ctx context.Context, blocks []core.Block, revision uint64) error {
	permit, err := s.persist.Acquire(ctx)
	if err != nil {
		return err
	}
	defer permit.Release()
	s.mu.Lock()
	snapshot := s.core.commands.Select(blocks, revision, false)
	state := s.core.checkpointStateLocked()
	s.mu.Unlock()
	commands, err := store.RestoreCommandStateHistory(snapshot)
	if err != nil {
		return err
	}
	update := store.CheckpointUpdate{
		State:         state,
		CommandState:  &snapshot,
		ChangedBlocks: []store.ChangedBlock{},
		BlockCount:    len(blocks),
	}
	for i, b := range blocks {
		update.ChangedBlocks = append(update.ChangedBlocks, store.ChangedBlock{Index: i, Block: b})
	}
	if err = s.deps.Store.Save(context.WithoutCancel(ctx), update, store.CommitGuard{}); err != nil {
		return err
	}
	s.mutate(func(_ *coreState) { s.adoptLocked(blocks, commands) })
	return nil
}

func (s *Session) adoptLocked(blocks []core.Block, commands *store.CommandStateHistory) {
	c := &s.core
	c.commands = commands
	c.generation++
	c.effects = append(c.effects, c.generationCancel)
	c.generationCtx, c.generationCancel = context.WithCancel(context.Background())
	batch := c.log.ReplaceAll(blocks)
	c.rows = transcript.DirtyRows{}
	s.eventLocked(&TranscriptChanged{Patches: batch.Patches, Revision: batch.Revision})
}
