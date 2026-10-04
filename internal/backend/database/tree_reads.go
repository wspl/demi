package database

import (
	"context"
	"database/sql"
	"slices"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/types"
)

// Summary reads the root's phase, output revision and latest terminal kind
// without loading its transcript. No root means EmptySummary.
func Summary(ctx context.Context, tx *sql.Tx) (SummaryFacts, error) {
	type rootRow struct {
		id       string
		state    store.CheckpointState
		count    int64
		revision uint64
	}
	root, found, err := queryRecord(
		ctx,
		tx,
		"nodes",
		"SELECT id,state,block_count,output_revision FROM nodes WHERE parent_id IS NULL",
		func(r *storedRow) rootRow {
			return rootRow{
				id:       r.text("id"),
				state:    storedJSON(r, "state", store.DecodeCheckpointState),
				count:    r.integer("block_count"),
				revision: r.count("output_revision"),
			}
		},
	)
	if err != nil {
		return SummaryFacts{}, err
	}
	if !found {
		return EmptySummary(), nil
	}
	terminal, ok, err := queryRecord(
		ctx,
		tx,
		"blocks",
		`SELECT block
FROM blocks
WHERE node_id=? AND idx<? AND json_extract(block,'$.type') IN ('response','error','abort')
ORDER BY idx DESC
LIMIT 1`,
		func(r *storedRow) types.Block { return storedJSON(r, "block", types.DecodeBlock) },
		root.id,
		root.count,
	)
	if err != nil {
		return SummaryFacts{}, err
	}
	facts := SummaryFacts{Phase: root.state.Phase, Revision: root.revision}
	if ok {
		facts.Last = summaryTerminal(&terminal)
	}
	return facts, nil
}

// HasRoot reports whether the tree has a root, a Fork's destination commit point.
func HasRoot(ctx context.Context, tx *sql.Tx) (bool, error) {
	var exists bool
	err := tx.QueryRowContext(ctx, "SELECT EXISTS (SELECT 1 FROM nodes WHERE parent_id IS NULL)").Scan(&exists)
	return exists, err
}

// ReadHistory reads root blocks followed by subagents depth first in spawn order.
func ReadHistory(ctx context.Context, tx *sql.Tx) (History, error) {
	type rootRow struct {
		id    types.NodeID
		count int64
	}
	root, found, err := queryRecord(
		ctx,
		tx,
		"nodes",
		"SELECT id,block_count FROM nodes WHERE parent_id IS NULL",
		func(r *storedRow) rootRow {
			return rootRow{id: checked(r, "id", types.ParseNodeID), count: r.integer("block_count")}
		},
	)
	history := History{Blocks: []types.Block{}, Subagents: []NodeHistory{}}
	if err != nil || !found {
		return history, err
	}
	history.Blocks, err = blocksOf(ctx, tx, root.id, root.count)
	if err != nil {
		return History{}, err
	}
	nodes, err := queryRecords(
		ctx,
		tx,
		"nodes",
		"SELECT * FROM nodes WHERE parent_id IS NOT NULL ORDER BY number",
		nodeRow,
	)
	if err != nil {
		return History{}, err
	}
	children := make(map[types.NodeID][]store.NodeRecord)
	for _, node := range nodes {
		children[*node.Parent] = append(children[*node.Parent], node)
	}
	pending := children[root.id]
	delete(children, root.id)
	slices.Reverse(pending)
	for len(pending) > 0 {
		node := pending[len(pending)-1]
		pending = pending[:len(pending)-1]
		var count int64
		if err := tx.QueryRowContext(ctx, "SELECT block_count FROM nodes WHERE id=?", node.ID).
			Scan(&count); err != nil {
			return History{}, err
		}
		blocks, err := blocksOf(ctx, tx, node.ID, count)
		if err != nil {
			return History{}, err
		}
		next := children[node.ID]
		delete(children, node.ID)
		slices.Reverse(next)
		pending = append(pending, next...)
		history.Subagents = append(history.Subagents, NodeHistory{Record: node, Blocks: blocks})
	}
	return history, nil
}

// summaryTerminal classifies only terminal blocks selected by the summary query.
func summaryTerminal(terminal *types.Block) *Terminal {
	var last *Terminal
	if terminal == nil {
		return nil
	}
	switch (*terminal).(type) {
	case *types.ResponseBlock:
		last = new(TerminalResponse)
	case *types.ErrorBlock:
		last = new(TerminalError)
	case *types.AbortBlock:
		last = new(TerminalAbort)
	case *types.AgentMessageBlock,
		*types.CompactionBoundaryBlock,
		*types.CompactionMarkerBlock,
		*types.ContextBlock,
		*types.RedactedThinkingBlock,
		*types.ResumeBlock,
		*types.SteerBlock,
		*types.TextBlock,
		*types.ThinkingBlock,
		*types.ToolCallBlock,
		*types.UserBlock,
		*types.WakeupBlock:
		// The SQL predicate excludes nonterminal blocks.
	}
	return last
}
