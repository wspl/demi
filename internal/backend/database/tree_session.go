package database

import (
	"context"
	"database/sql"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

// sessionStore owns a node's checkpoint operations through its conversation writer.
type sessionStore struct {
	tree *TreeStore
	node core.NodeID
}

// Blobs returns the conversation owner’s media store.
func (s *sessionStore) Blobs() store.BlobStore { return s.tree.blobs.Media() }

// Load reads a checkpoint in one conversation transaction.
func (s *sessionStore) Load(ctx context.Context) (*store.Checkpoint, error) {
	var checkpoint *store.Checkpoint
	err := s.tree.db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		checkpoint, err = readCheckpoint(ctx, tx, s.node)
		return err
	})
	return checkpoint, agentError(err)
}

// Save commits checkpoint changes and carried completions before notifying the tree.
func (s *sessionStore) Save(ctx context.Context, update store.CheckpointUpdate, guard store.CommitGuard) error {
	completions, err := update.CarriedCompletions()
	if err != nil {
		return err
	}
	commit := s.tree.db.CommitPoint()
	var due WakeupDue
	err = s.tree.db.call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		if err := guard.Check(); err != nil {
			return err
		}
		if err := writeCheckpoint(ctx, tx, s.tree.blobs, s.node, update, completions); err != nil {
			return err
		}
		due, err = earliestWakeup(ctx, tx)
		return err
	}, commit.Commit)
	if err != nil {
		return agentError(err)
	}
	s.tree.notify(s.node, due)
	return nil
}

func writeCheckpoint(
	ctx context.Context,
	tx *sql.Tx,
	blobs OwnerBlobs,
	node core.NodeID,
	update store.CheckpointUpdate,
	completions []core.CompletionID,
) error {
	touched, output, err := writeCheckpointBlocks(ctx, tx, node, update)
	if err != nil {
		return err
	}
	if err := writeCheckpointState(ctx, tx, node, update, output); err != nil {
		return err
	}
	if update.CommandState != nil {
		if err := writeCommandState(ctx, tx, node, *update.CommandState); err != nil {
			return err
		}
	}
	for _, round := range completions {
		if err := execSQL(
			ctx,
			tx,
			"UPDATE nodes SET delivered=1 WHERE id=? AND parent_id=? AND round=?",
			round.Child,
			node,
			round.Round,
		); err != nil {
			return err
		}
	}
	return blobs.CommitUses(ctx, touched)
}

func writeCheckpointBlocks(
	ctx context.Context,
	tx *sql.Tx,
	node core.NodeID,
	update store.CheckpointUpdate,
) ([]core.BlobRef, bool, error) {
	touched := make([]core.BlobRef, 0)
	output := false
	for _, change := range update.ChangedBlocks {
		if err := WriteBlock(ctx, tx, node, change.Index, change.Block, &touched); err != nil {
			return nil, false, err
		}
		switch change.Block.(type) {
		case *core.UserBlock,
			*core.ContextBlock,
			*core.WakeupBlock,
			*core.SteerBlock,
			*core.AgentMessageBlock,
			*core.ResumeBlock:
		case *core.AbortBlock,
			*core.CompactionBoundaryBlock,
			*core.CompactionMarkerBlock,
			*core.ErrorBlock,
			*core.RedactedThinkingBlock,
			*core.ResponseBlock,
			*core.TextBlock,
			*core.ThinkingBlock,
			*core.ToolCallBlock:
			output = true
		}
	}
	if err := TruncateBlocks(ctx, tx, node, update.BlockCount, &touched); err != nil {
		return nil, false, err
	}
	return touched, output, nil
}

func writeCheckpointState(
	ctx context.Context,
	tx *sql.Tx,
	node core.NodeID,
	update store.CheckpointUpdate,
	output bool,
) error {
	var revision *uint64
	if update.CommandState != nil {
		revision = new(update.CommandState.Revision)
	}
	wakeup, err := wakeupColumn(stateWakeup(update.State))
	if err != nil {
		return err
	}
	document, err := encoded(update.State)
	if err != nil {
		return err
	}
	changed, err := affected(
		ctx,
		tx,
		`UPDATE nodes
SET
    state=?,
    block_count=?,
    command_revision=COALESCE(?,command_revision),
    output_revision=output_revision+(CASE WHEN block_count>? OR ? THEN 1 ELSE 0 END),
    wakeup_at=(CASE WHEN parent_id IS NULL AND ? THEN NULL ELSE ? END)
WHERE id=?`,
		document,
		update.BlockCount,
		revision,
		update.BlockCount,
		output,
		update.State.Phase != core.SessionPhaseIdle,
		wakeup,
		node,
	)
	if err != nil {
		return err
	}
	if !changed {
		return missingNode(node)
	}
	return nil
}
