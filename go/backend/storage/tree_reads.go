package storage

import (
	"context"
	"database/sql"
	"errors"
	"fmt"

	"github.com/wspl/demi/go/core"
)

func blocksOf(ctx context.Context, db database, node core.NodeID, count int64) ([]core.Block, error) {
	rows, err := db.QueryContext(ctx, "SELECT idx,block FROM blocks WHERE node_id=? AND idx<? ORDER BY idx", node.String(), count)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	blocks := []core.Block{}
	gap := func() error {
		return &CorruptError{"blocks", "idx", fmt.Sprintf("node %s has no block row %d", node.String(), len(blocks))}
	}
	for rows.Next() {
		var index int64
		var text string
		if err = rows.Scan(&index, &text); err != nil {
			return nil, sqliteError(err)
		}
		if index != int64(len(blocks)) {
			return nil, gap()
		}
		block, err := core.Decode[core.Block]([]byte(text))
		if err != nil {
			return nil, corrupt("blocks", "block", err)
		}
		blocks = append(blocks, block)
	}
	if err = rows.Err(); err != nil {
		return nil, sqliteError(err)
	}
	if int64(len(blocks)) != count {
		return nil, gap()
	}
	return blocks, nil
}
func readCommandState(ctx context.Context, db database, node core.NodeID, revision uint64) (CommandStateSnapshot, error) {
	state := CommandStateSnapshot{Revision: revision, Versions: []CommandVersion{}, Boundaries: []SessionBoundary{}}
	rows, err := db.QueryContext(ctx, "SELECT revision,entries FROM command_snapshots WHERE node_id=? ORDER BY revision", node.String())
	if err != nil {
		return state, sqliteError(err)
	}
	for rows.Next() {
		var v CommandVersion
		var text string
		if err = rows.Scan(storedCount{"command_snapshots", "revision", &v.Revision}, &text); err != nil {
			rows.Close()
			return state, sqliteError(err)
		}
		v.Values, err = commandValues(text)
		if err != nil {
			rows.Close()
			return state, err
		}
		state.Versions = append(state.Versions, v)
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return state, sqliteError(err)
	}
	rows, err = db.QueryContext(ctx, "SELECT block_id,edge,command_revision FROM session_boundaries WHERE node_id=? ORDER BY block_id,edge", node.String())
	if err != nil {
		return state, sqliteError(err)
	}
	defer rows.Close()
	for rows.Next() {
		var v SessionBoundary
		var id string
		if err = rows.Scan(&id, &v.Edge, storedCount{"session_boundaries", "command_revision", &v.CommandRevision}); err != nil {
			return state, sqliteError(err)
		}
		v.BlockID, err = core.ParseBlockID(id)
		if err != nil {
			return state, corrupt("session_boundaries", "block_id", err)
		}
		if v.Edge != "before_user" && v.Edge != "after_assistant" && v.Edge != "after_block" {
			return state, &CorruptError{"session_boundaries", "edge", "unknown edge " + v.Edge}
		}
		state.Boundaries = append(state.Boundaries, v)
	}
	return state, sqliteError(rows.Err())
}
func (s *TreeStore) Load(ctx context.Context, node core.NodeID) (*Checkpoint, error) {
	var checkpoint *Checkpoint
	err := s.DB.transaction(ctx, func(tx *sql.Tx) error {
		var text string
		var count int64
		var revision uint64
		err := tx.QueryRowContext(ctx, "SELECT state,block_count,command_revision FROM nodes WHERE id=?", node.String()).Scan(&text, &count, &revision)
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err != nil {
			return sqliteError(err)
		}
		state, err := decodeState(text)
		if err != nil {
			return err
		}
		blocks, err := blocksOf(ctx, tx, node, count)
		if err != nil {
			return err
		}
		commands, err := readCommandState(ctx, tx, node, revision)
		if err != nil {
			return err
		}
		checkpoint = &Checkpoint{state, blocks, commands}
		return nil
	})
	return checkpoint, err
}

type SummaryFacts struct {
	Phase    core.SessionPhase
	Revision uint64
	Last     *string
}

func (d *ConversationDB) Summary(ctx context.Context) (SummaryFacts, error) {
	facts := SummaryFacts{Phase: core.SessionPhaseIdle}
	_, err := d.read(ctx, func(tx *sql.Tx) error {
		var id, text string
		var count int64
		err := tx.QueryRowContext(ctx, "SELECT id,state,block_count,output_revision FROM nodes WHERE parent_id IS NULL").Scan(&id, &text, &count, storedCount{"nodes", "output_revision", &facts.Revision})
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err != nil {
			return sqliteError(err)
		}
		state, err := decodeState(text)
		if err != nil {
			return err
		}
		facts.Phase = state.Phase
		err = tx.QueryRowContext(ctx, "SELECT block FROM blocks WHERE node_id=? AND idx<? AND json_extract(block,'$.type') IN ('response','error','abort') ORDER BY idx DESC LIMIT 1", id, count).Scan(&text)
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err != nil {
			return sqliteError(err)
		}
		block, err := core.Decode[core.Block]([]byte(text))
		if err != nil {
			return corrupt("blocks", "block", err)
		}
		var terminal string
		switch block.(type) {
		case core.BlockResponse:
			terminal = "response"
		case core.BlockError:
			terminal = "error"
		case core.BlockAbort:
			terminal = "abort"
		}
		facts.Last = &terminal
		return nil
	})
	return facts, err
}
func (d *ConversationDB) HasRoot(ctx context.Context) (bool, error) {
	var exists bool
	_, err := d.read(ctx, func(tx *sql.Tx) error {
		return sqliteError(tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM nodes WHERE parent_id IS NULL)").Scan(&exists))
	})
	return exists, err
}

type NodeHistory struct {
	Node   NodeRecord
	Blocks []core.Block
}
type History struct {
	Blocks    []core.Block
	Subagents []NodeHistory
}

func (d *ConversationDB) History(ctx context.Context) (History, error) {
	history := History{Blocks: []core.Block{}, Subagents: []NodeHistory{}}
	_, err := d.read(ctx, func(tx *sql.Tx) error {
		var root string
		var count int64
		err := tx.QueryRowContext(ctx, "SELECT id,block_count FROM nodes WHERE parent_id IS NULL").Scan(&root, &count)
		if errors.Is(err, sql.ErrNoRows) {
			return nil
		}
		if err != nil {
			return sqliteError(err)
		}
		id, err := core.ParseNodeID(root)
		if err != nil {
			return corrupt("nodes", "id", err)
		}
		history.Blocks, err = blocksOf(ctx, tx, id, count)
		if err != nil {
			return err
		}
		rows, err := tx.QueryContext(ctx, "SELECT "+nodeColumns+" FROM nodes WHERE parent_id IS NOT NULL ORDER BY number")
		if err != nil {
			return sqliteError(err)
		}
		children := map[core.NodeID][]NodeRecord{}
		for rows.Next() {
			node, err := scanNode(rows)
			if err != nil {
				rows.Close()
				return err
			}
			children[*node.Parent] = append(children[*node.Parent], *node)
		}
		err = rows.Err()
		rows.Close()
		if err != nil {
			return sqliteError(err)
		}
		var visit func(core.NodeID) error
		visit = func(parent core.NodeID) error {
			for _, node := range children[parent] {
				var count int64
				if err := tx.QueryRowContext(ctx, "SELECT block_count FROM nodes WHERE id=?", node.ID.String()).Scan(&count); err != nil {
					return sqliteError(err)
				}
				blocks, err := blocksOf(ctx, tx, node.ID, count)
				if err != nil {
					return err
				}
				history.Subagents = append(history.Subagents, NodeHistory{node, blocks})
				if err = visit(node.ID); err != nil {
					return err
				}
			}
			return nil
		}
		return visit(id)
	})
	return history, err
}
