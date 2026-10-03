package database

import (
	"context"
	"database/sql"
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/core"
)

// TreeStore is one conversation's agent tree in its database, with its owner's blobs.
type TreeStore struct {
	db    *ConversationDB
	blobs OwnerBlobs
	saved Saved
}

// NewTreeStore binds a conversation database, its owner's blobs and commit callback.
func NewTreeStore(db *ConversationDB, blobs OwnerBlobs, saved Saved) *TreeStore {
	return &TreeStore{db: db, blobs: blobs, saved: saved}
}

var _ store.TreeStore = (*TreeStore)(nil)

// Node returns a node's record, and false when it does not exist.
func (s *TreeStore) Node(ctx context.Context, id core.NodeID) (store.NodeRecord, bool, error) {
	var result *store.NodeRecord
	err := s.db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		record, found, err := nodeByID(ctx, tx, id)
		if found {
			result = &record
		}
		return err
	})
	if err != nil {
		return store.NodeRecord{}, false, agentError(err)
	}
	if result == nil {
		return store.NodeRecord{}, false, nil
	}
	return *result, true, nil
}

// Children returns direct children in number order, live and archived alike.
func (s *TreeStore) Children(ctx context.Context, parent core.NodeID) ([]store.NodeRecord, error) {
	var result []store.NodeRecord
	err := s.db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		result, err = childrenOf(ctx, tx, parent)
		return err
	})
	return result, agentError(err)
}

// CreateNode commits the record and first checkpoint; an existing node is refused.
func (s *TreeStore) CreateNode(ctx context.Context, record store.NodeRecord, initial store.CheckpointUpdate) error {
	completions, err := initial.CarriedCompletions()
	if err != nil {
		return err
	}
	var due WakeupDue
	err = s.db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		_, found, err := nodeByID(ctx, tx, record.ID)
		if err != nil {
			return err
		}
		if found {
			return fmt.Errorf("node %s already exists", record.ID)
		}
		if err := insertNode(ctx, tx, record, initial.State); err != nil {
			return err
		}
		if initial.CommandState == nil {
			initial.CommandState = new(store.InitialCommandState())
		}
		if err := writeCheckpoint(ctx, tx, s.blobs, record.ID, initial, completions); err != nil {
			return err
		}
		due, err = earliestWakeup(ctx, tx)
		return err
	})
	if err != nil {
		return agentError(err)
	}
	s.notify(record.ID, due)
	return nil
}

// SessionStore returns the node's checkpoint store. Saves also mark carried
// child completions delivered in the same commit.
func (s *TreeStore) SessionStore(id core.NodeID) store.SessionStore {
	return &sessionStore{tree: s, node: id}
}

// CloseNode closes a node after its final checkpoint, initially undelivered.
func (s *TreeStore) CloseNode(ctx context.Context, id core.NodeID, closed store.NodeClose) error {
	err := s.db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		phase, at, result, failure, err := closeColumns(&closed)
		if err != nil {
			return err
		}
		changed, err := affected(
			ctx,
			tx,
			"UPDATE nodes SET closed_phase=?,closed_at=?,result=?,failure=?,delivered=0 WHERE id=?",
			phase,
			at,
			result,
			failure,
			id,
		)
		if err != nil {
			return err
		}
		if !changed {
			return missingNode(id)
		}
		return nil
	})
	return agentError(err)
}

// ReopenNode starts a new round and queues its reviving message atomically.
func (s *TreeStore) ReopenNode(
	ctx context.Context,
	id core.NodeID,
	round uint64,
	startedAt core.Timestamp,
	message core.QueuedMessage,
) error {
	err := s.db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		state, found, err := nodeState(ctx, tx, id)
		if err != nil {
			return err
		}
		if !found {
			return missingNode(id)
		}
		state.Queue = []core.QueuedMessage{message}
		document, err := encoded(state)
		if err != nil {
			return err
		}
		at, err := startedAt.Millisecond()
		if err != nil {
			return err
		}
		return execSQL(
			ctx,
			tx,
			`UPDATE nodes
SET round=?,started_at=?,closed_phase=NULL,closed_at=NULL,result=NULL,failure=NULL,delivered=0,state=?
WHERE id=?`,
			round,
			at,
			document,
			id,
		)
	})
	return agentError(err)
}

// MarkDelivered marks only the named current round delivered.
func (s *TreeStore) MarkDelivered(ctx context.Context, id core.NodeID, round uint64) error {
	err := s.db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		_, found, err := nodeByID(ctx, tx, id)
		if err != nil {
			return err
		}
		if !found {
			return missingNode(id)
		}
		return execSQL(ctx, tx, "UPDATE nodes SET delivered=1 WHERE id=? AND round=?", id, round)
	})
	return agentError(err)
}

// DeleteNode deletes the node and all descendants with all their rows.
func (s *TreeStore) DeleteNode(ctx context.Context, id core.NodeID) error {
	var due WakeupDue
	err := s.db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		removed, err := SubtreeBlobs(ctx, tx, id)
		if err != nil {
			return err
		}
		if err := s.blobs.CommitUses(ctx, removed); err != nil {
			return err
		}
		if err := execSQL(ctx, tx, "DELETE FROM nodes WHERE id=?", id); err != nil {
			return err
		}
		due, err = earliestWakeup(ctx, tx)
		return err
	})
	if err != nil {
		return agentError(err)
	}
	s.notify(id, due)
	return nil
}

// NextNumber records the following number before returning this one.
func (s *TreeStore) NextNumber(ctx context.Context, sequence core.Sequence) (uint64, error) {
	var number uint64
	err := s.db.Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		number, err = NextNumber(ctx, tx, sequence)
		return err
	})
	return number, agentError(err)
}

// CommandOutput returns an ended command's output record, or nil if unknown.
func (s *TreeStore) CommandOutput(ctx context.Context, command core.CommandID) (store.StoredOutput, error) {
	var row CommandOutput
	var found bool
	_, err := s.db.Read(ctx, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		row, found, err = ReadCommandOutput(ctx, tx, command)
		return err
	})
	if err != nil {
		return nil, agentError(err)
	}
	if !found {
		return nil, nil
	}
	switch output := row.Output.(type) {
	case *OutputNotStored:
		return &store.OutputNotStored{Reason: output.Reason}, nil
	case *OutputRemoved:
		return &store.OutputRemoved{At: output.At}, nil
	case *OutputStored:
		data, found, err := s.blobs.Media().Read(ctx, output.Blob)
		if err != nil {
			return nil, err
		}
		if !found {
			return nil, fmt.Errorf("the blob %s of the output of %s is missing", output.Blob, command)
		}
		whole, err := remotehost.DecodeOutput(data, output.Missing)
		if err != nil {
			return nil, fmt.Errorf("%w: the output of %s does not decode: %w", store.ErrCorrupt, command, err)
		}
		return &store.OutputStored{Output: whole}, nil
	}
	return nil, nil
}

func agentError(err error) error {
	if errors.Is(err, ErrCorrupt) && !errors.Is(err, store.ErrCorrupt) {
		return fmt.Errorf("%w: %w", store.ErrCorrupt, err)
	}
	return err
}

func missingNode(id core.NodeID) error {
	return fmt.Errorf("no node %s", id)
}

func (s *TreeStore) notify(id core.NodeID, due WakeupDue) {
	if s.saved != nil {
		s.saved(id, due)
	}
}

func earliestWakeup(ctx context.Context, tx *sql.Tx) (WakeupDue, error) {
	row, _, err := queryRecord(
		ctx,
		tx,
		"nodes",
		"SELECT MIN(wakeup_at) AS wakeup_at FROM nodes",
		func(r *storedRow) WakeupDue { return rowWakeup(r, "wakeup_at") },
	)
	if err != nil {
		return nil, err
	}
	return row, nil
}

func stateWakeup(state store.CheckpointState) WakeupDue {
	var earliest *core.Timestamp
	for _, wakeup := range state.Wakeups {
		if wakeup.DueAt == nil {
			return &WakeupAtStart{}
		}
		if earliest == nil || *wakeup.DueAt < *earliest {
			earliest = wakeup.DueAt
		}
	}
	if earliest == nil {
		return nil
	}
	return &WakeupAt{At: *earliest}
}

func insertNode(ctx context.Context, tx *sql.Tx, record store.NodeRecord, state store.CheckpointState) error {
	phase, closed, result, failure, err := closeColumns(record.Closed)
	if err != nil {
		return err
	}
	at, err := record.StartedAt.Millisecond()
	if err != nil {
		return err
	}
	document, err := encoded(state)
	if err != nil {
		return err
	}
	if err := execSQL(
		ctx,
		tx,
		`INSERT INTO nodes (
    id,
    number,
    parent_id,
    description,
    profile,
    round,
    started_at,
    can_spawn,
    closed_phase,
    closed_at,
    result,
    failure,
    delivered,
    state,
    block_count,
    command_revision,
    output_revision
)
VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,0,0,0)`,
		record.ID,
		record.Number,
		record.Parent,
		record.Description,
		record.Profile,
		record.Round,
		at,
		record.CanSpawnSubagents,
		phase,
		closed,
		result,
		failure,
		record.Delivered,
		document,
	); err != nil {
		return err
	}
	return nil
}
