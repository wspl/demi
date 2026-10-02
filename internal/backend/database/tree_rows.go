package database

import (
	"context"
	"database/sql"
	"fmt"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

// nodeRow checks a node's identity and its close-dependent columns.
func nodeRow(r *storedRow) store.NodeRecord {
	n := store.NodeRecord{ID: checked(r, "id", core.ParseNodeID), Number: r.count("number"), Parent: optionalChecked(r, "parent_id", core.ParseNodeID), Description: r.text("description"), Profile: r.optionalText("profile"), Round: r.count("round"), StartedAt: r.instant("started_at"), CanSpawnSubagents: r.boolean("can_spawn"), Delivered: r.boolean("delivered")}
	if phase := r.optionalText("closed_phase"); phase != nil {
		var closePhase store.ClosePhase
		switch *phase {
		case "completed":
			closePhase = &store.Completed{Result: closeText(r, "result")}
		case "aborted":
			closePhase = &store.Aborted{}
		case "error":
			closePhase = &store.Failed{Failure: closeText(r, "failure")}
		default:
			r.bad("closed_phase", fmt.Errorf("unknown close phase %s", *phase))
		}
		if r.values["closed_at"] == nil {
			r.bad("closed_at", fmt.Errorf("a closed node has no close time"))
		}
		n.Closed = &store.NodeClose{Phase: closePhase, At: r.instant("closed_at")}
	}
	return n
}
func closeText(r *storedRow, column string) string {
	value := r.optionalText(column)
	if value == nil {
		r.bad(column, fmt.Errorf("the close phase requires it"))
		return ""
	}
	return *value
}
func closeColumns(closed *store.NodeClose) (phase *string, at *int64, result *string, failure *string, err error) {
	if closed == nil {
		return
	}
	value, err := closed.At.Millisecond()
	if err != nil {
		return nil, nil, nil, nil, err
	}
	at = &value
	switch p := closed.Phase.(type) {
	case *store.Completed:
		phase = new("completed")
		result = &p.Result
	case *store.Aborted:
		phase = new("aborted")
	case *store.Failed:
		phase = new("error")
		failure = &p.Failure
	}
	return
}
func nodeByID(ctx context.Context, tx *sql.Tx, id core.NodeID) (*store.NodeRecord, error) {
	return queryRecord(ctx, tx, "nodes", "SELECT * FROM nodes WHERE id=?", nodeRow, id)
}
func childrenOf(ctx context.Context, tx *sql.Tx, parent core.NodeID) ([]store.NodeRecord, error) {
	return queryRecords(ctx, tx, "nodes", "SELECT * FROM nodes WHERE parent_id=? ORDER BY number", nodeRow, parent)
}
func nodeState(ctx context.Context, tx *sql.Tx, id core.NodeID) (*store.CheckpointState, error) {
	return queryRecord(ctx, tx, "nodes", "SELECT state FROM nodes WHERE id=?", func(r *storedRow) store.CheckpointState { return storedJSON(r, "state", store.DecodeCheckpointState) }, id)
}
func blocksOf(ctx context.Context, tx *sql.Tx, node core.NodeID, count int64) ([]core.Block, error) {
	expected := int64(0)
	blocks, err := queryRecords(ctx, tx, "blocks", "SELECT idx,block FROM blocks WHERE node_id=? AND idx<? ORDER BY idx", func(r *storedRow) core.Block {
		if r.integer("idx") != expected {
			r.bad("idx", fmt.Errorf("node %s has no block row %d", node, expected))
		}
		expected++
		return storedJSON(r, "block", core.DecodeBlock)
	}, node, count)
	if err != nil {
		return nil, err
	}
	if int64(len(blocks)) != count {
		return nil, &Error{Kind: Corrupt, Table: "blocks", Column: "idx", Reason: fmt.Sprintf("node %s has no block row %d", node, len(blocks))}
	}
	return blocks, nil
}
func readCheckpoint(ctx context.Context, tx *sql.Tx, node core.NodeID) (*store.Checkpoint, error) {
	type stateRow struct {
		state    store.CheckpointState
		count    int64
		revision uint64
	}
	row, err := queryRecord(ctx, tx, "nodes", "SELECT state,block_count,command_revision FROM nodes WHERE id=?", func(r *storedRow) stateRow {
		return stateRow{state: storedJSON(r, "state", store.DecodeCheckpointState), count: r.integer("block_count"), revision: r.count("command_revision")}
	}, node)
	if err != nil || row == nil {
		return nil, err
	}
	blocks, err := blocksOf(ctx, tx, node, row.count)
	if err != nil {
		return nil, err
	}
	commands, err := readCommandState(ctx, tx, node, row.revision)
	if err != nil {
		return nil, err
	}
	return &store.Checkpoint{State: row.state, Transcript: blocks, CommandState: commands}, nil
}
