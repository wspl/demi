package database

import (
	"context"
	"database/sql"
	"slices"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
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
	root, err := queryRecord(
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
	if root == nil {
		return EmptySummary(), nil
	}
	terminal, err := queryRecord(
		ctx,
		tx,
		"blocks",
		`SELECT block
FROM blocks
WHERE node_id=? AND idx<? AND json_extract(block,'$.type') IN ('response','error','abort')
ORDER BY idx DESC
LIMIT 1`,
		func(r *storedRow) core.Block { return storedJSON(r, "block", core.DecodeBlock) },
		root.id,
		root.count,
	)
	if err != nil {
		return SummaryFacts{}, err
	}
	facts := SummaryFacts{Phase: root.state.Phase, Revision: root.revision}
	facts.Last = summaryTerminal(terminal)
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
		id    core.NodeID
		count int64
	}
	root, err := queryRecord(
		ctx,
		tx,
		"nodes",
		"SELECT id,block_count FROM nodes WHERE parent_id IS NULL",
		func(r *storedRow) rootRow {
			return rootRow{id: checked(r, "id", core.ParseNodeID), count: r.integer("block_count")}
		},
	)
	history := History{Blocks: []core.Block{}, Subagents: []NodeHistory{}}
	if err != nil || root == nil {
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
	children := make(map[core.NodeID][]store.NodeRecord)
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
func summaryTerminal(terminal *core.Block) *Terminal {
	var last *Terminal
	if terminal == nil {
		return nil
	}
	switch (*terminal).(type) {
	case *core.ResponseBlock:
		last = new(TerminalResponse)
	case *core.ErrorBlock:
		last = new(TerminalError)
	case *core.AbortBlock:
		last = new(TerminalAbort)
	case *core.AgentMessageBlock,
		*core.CompactionBoundaryBlock,
		*core.CompactionMarkerBlock,
		*core.ContextBlock,
		*core.RedactedThinkingBlock,
		*core.ResumeBlock,
		*core.SteerBlock,
		*core.TextBlock,
		*core.ThinkingBlock,
		*core.ToolCallBlock,
		*core.UserBlock,
		*core.WakeupBlock:
		// The SQL predicate excludes nonterminal blocks.
	}
	return last
}
