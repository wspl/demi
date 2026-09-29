package storage

import (
	"context"
	"database/sql"
	"encoding/json/v2"
	"errors"
	"fmt"

	"github.com/wspl/demi/go/core"
)

type TreeStore struct {
	DB    *ConversationDB
	Blobs *UserBlobs
	Saved func(core.NodeID)
}

const nodeColumns = "id,number,parent_id,description,profile,round,started_at,can_spawn,closed_phase,closed_at,result,failure,delivered"

func scanNode(row scanner) (*NodeRecord, error) {
	var r NodeRecord
	var id string
	var parent, phase *string
	var closed *int64
	var result, failure *string
	var ms int64
	err := row.Scan(&id, storedCount{"nodes", "number", &r.Number}, &parent, &r.Description, &r.Profile, storedCount{"nodes", "round", &r.Round}, &ms, &r.CanSpawnSubagents, &phase, &closed, &result, &failure, &r.Delivered)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	r.ID, err = core.ParseNodeID(id)
	if err != nil {
		return nil, corrupt("nodes", "id", err)
	}
	if parent != nil {
		value, err := core.ParseNodeID(*parent)
		if err != nil {
			return nil, corrupt("nodes", "parent_id", err)
		}
		r.Parent = &value
	}
	r.StartedAt, err = instant("nodes", "started_at", ms)
	if err != nil {
		return nil, err
	}
	if phase != nil {
		switch *phase {
		case "completed":
			if result == nil {
				return nil, &CorruptError{"nodes", "result", "the close phase requires it"}
			}
		case "error":
			if failure == nil {
				return nil, &CorruptError{"nodes", "failure", "the close phase requires it"}
			}
		case "aborted":
		default:
			return nil, &CorruptError{"nodes", "closed_phase", "unknown close phase " + *phase}
		}
		if closed == nil {
			return nil, &CorruptError{"nodes", "closed_at", "a closed node has no close time"}
		}
		at, err := instant("nodes", "closed_at", *closed)
		if err != nil {
			return nil, err
		}
		r.Closed = &NodeClose{*phase, at, result, failure}
	}
	return &r, nil
}
func closeColumns(close *NodeClose) (phase *string, at *int64, result, failure *string) {
	if close == nil {
		return
	}
	ms := close.At.Millisecond()
	return &close.Phase, &ms, close.Result, close.Failure
}
func (s *TreeStore) Node(ctx context.Context, id core.NodeID) (*NodeRecord, error) {
	db, release, err := s.DB.writer(ctx)
	if err != nil {
		return nil, err
	}
	defer release()
	return scanNode(db.QueryRowContext(ctx, "SELECT "+nodeColumns+" FROM nodes WHERE id=?", id.String()))
}
func (s *TreeStore) Children(ctx context.Context, parent core.NodeID) ([]NodeRecord, error) {
	db, release, err := s.DB.writer(ctx)
	if err != nil {
		return nil, err
	}
	defer release()
	rows, err := db.QueryContext(ctx, "SELECT "+nodeColumns+" FROM nodes WHERE parent_id=? ORDER BY number", parent.String())
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	nodes := []NodeRecord{}
	for rows.Next() {
		node, err := scanNode(rows)
		if err != nil {
			return nil, err
		}
		nodes = append(nodes, *node)
	}
	return nodes, sqliteError(rows.Err())
}
func (s *TreeStore) CreateNode(ctx context.Context, node NodeRecord, initial CheckpointUpdate) error {
	rounds, err := initial.completions()
	if err != nil {
		return err
	}
	if initial.CommandState == nil {
		state := initialCommandState()
		initial.CommandState = &state
	}
	err = s.DB.transaction(ctx, func(tx *sql.Tx) error {
		var exists bool
		if err := tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM nodes WHERE id=?)", node.ID.String()).Scan(&exists); err != nil {
			return sqliteError(err)
		}
		if exists {
			return fmt.Errorf("node %s already exists", node.ID.String())
		}
		text, err := json.Marshal(initial.State)
		if err != nil {
			return err
		}
		var parent *string
		if node.Parent != nil {
			value := node.Parent.String()
			parent = &value
		}
		phase, at, result, failure := closeColumns(node.Closed)
		_, err = tx.ExecContext(ctx, "INSERT INTO nodes("+nodeColumns+",state,block_count,command_revision,output_revision) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,0,0,0)", node.ID.String(), node.Number, parent, node.Description, node.Profile, node.Round, node.StartedAt.Millisecond(), node.CanSpawnSubagents, phase, at, result, failure, node.Delivered, string(text))
		if err != nil {
			return sqliteError(err)
		}
		return s.writeCheckpoint(ctx, tx, node.ID, initial, rounds)
	})
	if err == nil && s.Saved != nil {
		s.Saved(node.ID)
	}
	return err
}
func (s *TreeStore) Save(ctx context.Context, node core.NodeID, update CheckpointUpdate, guard func() error) error {
	rounds, err := update.completions()
	if err != nil {
		return err
	}
	err = s.DB.transaction(ctx, func(tx *sql.Tx) error {
		if guard != nil {
			if err := guard(); err != nil {
				return err
			}
		}
		return s.writeCheckpoint(ctx, tx, node, update, rounds)
	})
	if err == nil && s.Saved != nil {
		s.Saved(node)
	}
	return err
}
func (s *TreeStore) writeCheckpoint(ctx context.Context, tx *sql.Tx, node core.NodeID, u CheckpointUpdate, rounds []core.CompletionID) error {
	touched := []core.BlobRef{}
	output := false
	for _, change := range u.ChangedBlocks {
		if err := writeBlock(ctx, tx, node, change.Index, change.Block, &touched); err != nil {
			return err
		}
		switch change.Block.(type) {
		case core.BlockUser, core.BlockContext, core.BlockWakeup, core.BlockSteer, core.BlockAgentMessage, core.BlockResume:
		default:
			output = true
		}
	}
	removed, err := queryBlobs(ctx, tx, "DELETE FROM blob_refs WHERE node_id=? AND idx>=? RETURNING blob", node.String(), u.BlockCount)
	if err != nil {
		return err
	}
	touched = append(touched, removed...)
	if _, err = tx.ExecContext(ctx, "DELETE FROM blocks WHERE node_id=? AND idx>=?", node.String(), u.BlockCount); err != nil {
		return sqliteError(err)
	}
	state, err := json.Marshal(u.State)
	if err != nil {
		return err
	}
	var revision *uint64
	if u.CommandState != nil {
		revision = &u.CommandState.Revision
	}
	result, err := tx.ExecContext(ctx, `UPDATE nodes SET state=?2,block_count=?3,command_revision=COALESCE(?5,command_revision),output_revision=output_revision+(CASE WHEN block_count>?3 OR ?4 THEN 1 ELSE 0 END) WHERE id=?1`, node.String(), string(state), u.BlockCount, output, revision)
	if err != nil {
		return sqliteError(err)
	}
	count, err := result.RowsAffected()
	if err != nil {
		return sqliteError(err)
	}
	if count == 0 {
		return fmt.Errorf("no node %s", node.String())
	}
	if u.CommandState != nil {
		if err = writeCommandState(ctx, tx, node, *u.CommandState); err != nil {
			return err
		}
	}
	for _, round := range rounds {
		if _, err = tx.ExecContext(ctx, "UPDATE nodes SET delivered=1 WHERE id=? AND parent_id=? AND round=?", round.Child.String(), node.String(), round.Round); err != nil {
			return sqliteError(err)
		}
	}
	return s.Blobs.CommitUses(touched)
}
func writeCommandState(ctx context.Context, tx *sql.Tx, node core.NodeID, state CommandStateSnapshot) error {
	revisions := make([]uint64, 0, len(state.Versions))
	for _, version := range state.Versions {
		revisions = append(revisions, version.Revision)
		var text string
		err := tx.QueryRowContext(ctx, "SELECT entries FROM command_snapshots WHERE node_id=? AND revision=?", node.String(), version.Revision).Scan(&text)
		if err == nil {
			stored, err := commandValues(text)
			if err != nil {
				return err
			}
			equal, err := commandValuesEqual(stored, version.Values)
			if err != nil {
				return err
			}
			if !equal {
				return fmt.Errorf("command-state version %d is immutable", version.Revision)
			}

		} else if errors.Is(err, sql.ErrNoRows) {
			bytes, err := json.Marshal(version.Values, json.Deterministic(true))
			if err != nil {
				return err
			}
			if _, err = commandValues(string(bytes)); err != nil {
				return err
			}
			if _, err = tx.ExecContext(ctx, "INSERT INTO command_snapshots(node_id,revision,entries) VALUES (?,?,?)", node.String(), version.Revision, string(bytes)); err != nil {
				return sqliteError(err)
			}
		} else {
			return sqliteError(err)
		}
	}
	if _, err := tx.ExecContext(ctx, "DELETE FROM session_boundaries WHERE node_id=?", node.String()); err != nil {
		return sqliteError(err)
	}
	bytes, err := json.Marshal(revisions)
	if err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, "DELETE FROM command_snapshots WHERE node_id=? AND revision NOT IN (SELECT value FROM json_each(?))", node.String(), string(bytes)); err != nil {
		return sqliteError(err)
	}
	for _, boundary := range state.Boundaries {
		if _, err = tx.ExecContext(ctx, "INSERT INTO session_boundaries(node_id,block_id,edge,command_revision) VALUES (?,?,?,?)", node.String(), boundary.BlockID.String(), boundary.Edge, boundary.CommandRevision); err != nil {
			return sqliteError(err)
		}
	}
	return nil
}
func (s *TreeStore) CloseNode(ctx context.Context, node core.NodeID, close NodeClose) error {
	return s.DB.transaction(ctx, func(tx *sql.Tx) error {
		phase, at, result, failure := closeColumns(&close)
		changed, err := tx.ExecContext(ctx, "UPDATE nodes SET closed_phase=?,closed_at=?,result=?,failure=?,delivered=0 WHERE id=?", phase, at, result, failure, node.String())
		if err != nil {
			return sqliteError(err)
		}
		count, err := changed.RowsAffected()
		if err != nil {
			return sqliteError(err)
		}
		if count == 0 {
			return fmt.Errorf("no node %s", node.String())
		}
		return nil
	})
}
func (s *TreeStore) ReopenNode(ctx context.Context, node core.NodeID, round uint64, started core.Timestamp, message core.QueuedMessage) error {
	return s.DB.transaction(ctx, func(tx *sql.Tx) error {
		var text string
		err := tx.QueryRowContext(ctx, "SELECT state FROM nodes WHERE id=?", node.String()).Scan(&text)
		if errors.Is(err, sql.ErrNoRows) {
			return fmt.Errorf("no node %s", node.String())
		}
		if err != nil {
			return sqliteError(err)
		}
		state, err := decodeState(text)
		if err != nil {
			return err
		}
		state.Queue = []core.QueuedMessage{message}
		data, err := json.Marshal(state)
		if err != nil {
			return err
		}
		_, err = tx.ExecContext(ctx, "UPDATE nodes SET round=?,started_at=?,closed_phase=NULL,closed_at=NULL,result=NULL,failure=NULL,delivered=0,state=? WHERE id=?", round, started.Millisecond(), string(data), node.String())
		return sqliteError(err)
	})
}
func (s *TreeStore) MarkDelivered(ctx context.Context, node core.NodeID, round uint64) error {
	return s.DB.transaction(ctx, func(tx *sql.Tx) error {
		var exists bool
		if err := tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM nodes WHERE id=?)", node.String()).Scan(&exists); err != nil {
			return sqliteError(err)
		}
		if !exists {
			return fmt.Errorf("no node %s", node.String())
		}
		_, err := tx.ExecContext(ctx, "UPDATE nodes SET delivered=1 WHERE id=? AND round=?", node.String(), round)
		return sqliteError(err)
	})
}
func (s *TreeStore) DeleteNode(ctx context.Context, node core.NodeID) error {
	return s.DB.transaction(ctx, func(tx *sql.Tx) error {
		touched, err := queryBlobs(ctx, tx, `WITH RECURSIVE subtree(id) AS(SELECT ?1 UNION SELECT nodes.id FROM nodes JOIN subtree ON nodes.parent_id=subtree.id) SELECT blob FROM blob_refs WHERE node_id IN subtree`, node.String())
		if err != nil {
			return err
		}
		if err = s.Blobs.CommitUses(touched); err != nil {
			return err
		}
		_, err = tx.ExecContext(ctx, "DELETE FROM nodes WHERE id=?", node.String())
		return sqliteError(err)
	})
}
