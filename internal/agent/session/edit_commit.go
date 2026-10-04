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

func (c *coreState) checkEditLocked(
	operation core.OperationID,
	digest string,
	version framewire.TranscriptVersion,
) (EditCheck, error) {
	for _, receipt := range c.edits {
		if receipt.OperationID == operation {
			if receipt.Digest != digest {
				return nil, ErrEditConflict
			}
			return &EditAccepted{Receipt: receipt}, nil
		}
	}
	if c.edit != nil && c.edit.submission.OperationID == operation {
		if c.edit.submission.Digest != digest {
			return nil, ErrEditConflict
		}
		return &EditInFlight{acceptance: c.edit.acceptance}, nil
	}
	if c.disposing {
		return nil, ErrClosed
	}
	if c.statusLocked().Settle != Settled || c.edit != nil || len(c.inputs) > 0 || len(c.wakeups) > 0 {
		return nil, ErrEditBusy
	}
	if version != c.log.Version() {
		return nil, ErrEditStale
	}
	return &EditProceed{}, nil
}

func (s *Session) editAndSend(ctx context.Context, submission EditSubmission) (store.EditReceipt, error) {
	var check EditCheck
	var err error
	s.mutate(func(c *coreState) {
		check, err = c.checkEditLocked(submission.OperationID, submission.Digest, submission.Version)
		if err != nil {
			return
		}
		if _, ok := check.(*EditProceed); !ok {
			return
		}
		acceptance := newAcceptance()
		c.edit = &editFlight{submission: submission, acceptance: acceptance}
		check = &EditInFlight{acceptance: acceptance}
		c.queue = append(c.queue, &action{kind: editAction, turn: core.TurnID(s.deps.IDs.NextID())})
		s.startNextLocked()
	})
	if err != nil {
		return store.EditReceipt{}, err
	}
	switch v := check.(type) {
	case *EditAccepted:
		return v.Receipt, nil
	case *EditInFlight:
		return v.acceptance.wait(ctx)
	case *EditProceed:
	}
	return store.EditReceipt{}, ErrClosed
}

func (s *Session) rejectEditLocked(err error) {
	c := &s.core
	if c.edit == nil || c.edit.accepted {
		return
	}
	acceptance := c.edit.acceptance
	c.edit = nil
	c.effects = append(c.effects, func() { acceptance.finish(store.EditReceipt{}, err) })
	if c.waitingChange != nil {
		c.replaceSwitchLocked(&c.change, *c.waitingChange)
		c.waitingChange = nil
	}
}

type editCandidate struct {
	blocks   []core.Block
	commands *store.CommandStateHistory
	model    core.ModelSelection
	receipt  store.EditReceipt
	update   store.CheckpointUpdate
	source   provider.Runtime
}

func (s *Session) prepareEditLocked(preamble *string) (editCandidate, error) {
	c := &s.core
	submission := c.edit.submission
	prefix, err := transcript.BeforeUser(c.log.Blocks(), submission.Target)
	if err != nil {
		return editCandidate{}, err
	}
	target, ok := c.log.Find(submission.Target).(*core.UserBlock)
	if !ok {
		return editCandidate{}, transcript.ErrNotUserMessage
	}
	content, err := resolveEdit(submission.Content, target.Content)
	if err != nil {
		return editCandidate{}, err
	}
	revision, ok := c.commands.Boundary(submission.Target, store.BeforeUser)
	if !ok {
		//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
		return editCandidate{}, fmt.Errorf("No command-state boundary before %s", submission.Target)
	}
	model := c.model
	source := c.provider
	if c.change != nil {
		model = c.change.Model
		if c.change.Runtime != nil {
			source = c.change.Runtime
		}
	}
	candidateLog := transcript.NewLog(prefix, s.deps.IDs, s.deps.Clock)
	user := candidateLog.PushUser(c.active.turn, model, content, preamble)
	blocks := candidateLog.Blocks()
	commands, err := store.RestoreCommandStateHistory(c.commands.Select(blocks, revision, false))
	if err != nil {
		return editCandidate{}, err
	}
	commands.Capture(user, store.BeforeUser, revision)
	commands.Capture(user, store.AfterBlock, revision)
	receipt := store.EditReceipt{OperationID: submission.OperationID, Digest: submission.Digest, TurnID: c.active.turn}
	update := c.editUpdateLocked(model, receipt, commands, blocks)
	return editCandidate{
		blocks:   blocks,
		commands: commands,
		model:    model,
		receipt:  receipt,
		update:   update,
		source:   source,
	}, nil
}

func (s *Session) runEdit(ctx context.Context) error {
	reservation, err := s.deps.Runtime.ReserveEdit(ctx)
	if reservation != nil {
		defer reservation.Release()
	}
	if err != nil {
		return err
	}
	if err = s.Flush(ctx); err != nil {
		return err
	}
	preamble, err := s.deps.Runtime.Preamble(ctx)
	if err != nil {
		return err
	}
	if err = ctx.Err(); err != nil {
		return err
	}
	if _, err = s.useProvider(ctx); err != nil {
		return err
	}
	reserved := true
	defer func() {
		if reserved {
			s.mutate(func(c *coreState) { c.providerBusy = false })
		}
	}()
	s.mu.Lock()
	candidate, err := s.prepareEditLocked(preamble)
	s.mu.Unlock()
	if err != nil {
		return err
	}
	committed, err := s.commitEdit(ctx, candidate)
	if !committed {
		return err
	}
	s.mutate(func(c *coreState) { c.providerBusy = false })
	reserved = false
	if reservation != nil {
		reservation.Release()
	}
	if err != nil {
		return err
	}
	if err = ctx.Err(); err != nil {
		return err
	}
	if err = s.preflight(ctx); err != nil {
		return err
	}
	return s.runTurn(ctx, false)
}

// resolveEdit resolves kept media solely against the edited user message.
func resolveEdit(parts []EditContent, target []core.UserContentBlock) ([]core.UserContentBlock, error) {
	result := make([]core.UserContentBlock, 0, len(parts))
	for _, part := range parts {
		switch part := part.(type) {
		case *Content:
			result = append(result, part.Block)
		case *KeptAttachment:
			var found core.UserContentBlock
			for _, block := range target {
				if a, ok := block.(*core.UserAttachment); ok && a.Path == part.Path {
					found = block
					break
				}
			}
			if found == nil {
				//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
				return nil, fmt.Errorf("The edited message holds no attachment at %s", part.Path)
			}
			result = append(result, found)
		case *KeptMedia:
			found, err := resolveKeptMedia(part.Media, target)
			if err != nil {
				return nil, err
			}
			result = append(result, found)
		}
	}
	return result, nil
}

func (c *coreState) editUpdateLocked(
	model core.ModelSelection,
	receipt store.EditReceipt,
	commands *store.CommandStateHistory,
	blocks []core.Block,
) store.CheckpointUpdate {
	state := c.checkpointStateLocked()
	state.Phase = "running"
	state.Model = model
	state.Queue = []core.QueuedMessage{}
	state.Edits = append(state.Edits, receipt)
	snapshot := commands.Snapshot(nil)
	update := store.CheckpointUpdate{
		State:         state,
		CommandState:  &snapshot,
		BlockCount:    len(blocks),
		ChangedBlocks: []store.ChangedBlock{},
	}
	for i, b := range blocks {
		update.ChangedBlocks = append(update.ChangedBlocks, store.ChangedBlock{Index: i, Block: b})
	}
	return update
}

func (s *Session) commitEdit(ctx context.Context, candidate editCandidate) (bool, error) {
	fresh := candidate.source.Fresh()
	accepted := false
	defer func() {
		if !accepted {
			if closeErr := fresh.Close(context.WithoutCancel(ctx)); closeErr != nil {
				s.emit(&ErrorEvent{Report: ReportError{Message: closeErr.Error()}})
			}
		}
	}()
	permit, err := s.persist.Acquire(ctx)
	if err != nil {
		return false, err
	}
	err = ctx.Err()
	if err == nil {
		err = s.deps.Store.Save(context.WithoutCancel(ctx), candidate.update, store.CommitGuard{})
	}
	if err != nil {
		permit.Release()
		return false, err
	}
	var retired []provider.Runtime
	s.mutate(func(c *coreState) {
		s.adoptLocked(candidate.blocks, candidate.commands)
		c.edits = append(c.edits, candidate.receipt)
		retired = append(c.retired, c.provider)
		if c.change != nil && c.change.Runtime != nil {
			retired = append(retired, c.change.Runtime)
		}
		c.retired = nil
		c.provider = fresh
		c.model = candidate.model
		c.change = c.waitingChange
		c.waitingChange = nil
		c.edit.accepted = true
		s.eventLocked(&EditCommitted{Receipt: candidate.receipt})
		acceptance := c.edit.acceptance
		c.effects = append(c.effects, func() { acceptance.finish(candidate.receipt, nil) })
	})
	accepted = true
	permit.Release()
	for _, old := range retired {
		err = errors.Join(err, old.Close(context.WithoutCancel(ctx)))
	}
	return true, err
}

func resolveKeptMedia(media framewire.MediaRef, target []core.UserContentBlock) (core.UserContentBlock, error) {
	var kind string
	var blob core.BlobRef
	switch ref := media.(type) {
	case *framewire.MediaImageRef:
		kind = "image"
		blob = ref.Ref
	case *framewire.MediaVideoRef:
		kind = "video"
		blob = ref.Ref
	case *framewire.MediaDocumentRef:
		kind = "document"
		blob = ref.Ref
	}
	var found core.UserContentBlock
	for _, block := range target {
		var match bool
		switch b := block.(type) {
		case *core.UserImage:
			if ref, ok := b.Source.(*core.MediaSourceRef); ok {
				match = kind == "image" && ref.Ref == blob
			}
		case *core.UserVideo:
			if ref, ok := b.Source.(*core.MediaSourceRef); ok {
				match = kind == "video" && ref.Ref == blob
			}
		case *core.UserDocument:
			if ref, ok := b.Source.(*core.DocumentRef); ok {
				match = kind == "document" && ref.Ref == blob
			}
		case *core.UserText, *core.UserAttachment, *core.UserReference:
		}
		if match {
			found = block
			break
		}
	}
	if found == nil {
		//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
		return nil, fmt.Errorf("The edited message holds no %s %s", kind, blob)
	}
	return found, nil
}
