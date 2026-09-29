package storage

import (
	"context"
	"database/sql"

	"github.com/wspl/demi/go/core"
)

type Retirement struct {
	Now  core.Timestamp
	Idle bool
}
type RetiredNode struct {
	Node   core.NodeID
	Blocks []BlockChange
}

// RetiredBlocks ports the persisted transcript rule until G7d provides its owner.
func RetiredBlocks(blocks []core.Block, r Retirement) []BlockChange {
	start := 0
	summarized := false
	for i, block := range blocks {
		if boundary, ok := block.(core.BlockCompactionBoundary); ok {
			start = i
			summarized = r.Now.Millisecond()-boundary.CreatedAt.Millisecond() > 86400000
		}
	}
	changed := []BlockChange{}
	for i, block := range blocks {
		call, ok := block.(core.BlockToolCall)
		if !ok || r.Now.Millisecond()-call.CreatedAt.Millisecond() <= 2592000000 || !(r.Idle || (summarized && i < start)) {
			continue
		}
		output := append([]core.ToolResultContentBlock(nil), call.Output...)
		any := false
		for part, value := range output {
			var source core.ToolMediaSource
			var kind core.ModelMediaKind
			switch v := value.(type) {
			case core.ToolResultContentBlockImage:
				source = v.Source
				kind = core.ModelMediaKindImage
			case core.ToolResultContentBlockVideo:
				source = v.Source
				kind = core.ModelMediaKindVideo
			}
			if ref, ok := source.(core.ToolMediaSourceRef); ok {
				output[part] = core.ToolResultContentBlockGone{Kind: kind, MediaType: ref.MediaType, Cause: core.GoneCauseRetired{At: r.Now}}
				any = true
			}
		}
		if any {
			call.Output = output
			changed = append(changed, BlockChange{i, call})
		}
	}
	return changed
}
func retirable(ctx context.Context, db database, r Retirement) ([]RetiredNode, error) {
	rows, err := db.QueryContext(ctx, "SELECT id,block_count FROM nodes WHERE id IN (SELECT node_id FROM blob_refs WHERE holder='tool_result' AND at<?) ORDER BY id", r.Now.Millisecond()-2592000000)
	if err != nil {
		return nil, sqliteError(err)
	}
	type candidate struct {
		node  core.NodeID
		count int64
	}
	candidates := []candidate{}
	for rows.Next() {
		var id string
		var count int64
		if err = rows.Scan(&id, &count); err != nil {
			rows.Close()
			return nil, sqliteError(err)
		}
		node, err := core.ParseNodeID(id)
		if err != nil {
			rows.Close()
			return nil, corrupt("nodes", "id", err)
		}
		candidates = append(candidates, candidate{node, count})
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return nil, sqliteError(err)
	}
	retired := []RetiredNode{}
	for _, candidate := range candidates {
		blocks, err := blocksOf(ctx, db, candidate.node, candidate.count)
		if err != nil {
			return nil, err
		}
		if changed := RetiredBlocks(blocks, r); len(changed) > 0 {
			retired = append(retired, RetiredNode{candidate.node, changed})
		}
	}
	return retired, nil
}
func (d *ConversationDB) Retirable(ctx context.Context, r Retirement) ([]RetiredNode, error) {
	var retired []RetiredNode
	_, err := d.read(ctx, func(tx *sql.Tx) error {
		var err error
		retired, err = retirable(ctx, tx, r)
		return err
	})
	return retired, err
}
func (d *ConversationDB) Retire(ctx context.Context, blobs *UserBlobs, r Retirement) (int, error) {
	count := 0
	err := d.transaction(ctx, func(tx *sql.Tx) error {
		retired, err := retirable(ctx, tx, r)
		if err != nil {
			return err
		}
		touched := []core.BlobRef{}
		for _, node := range retired {
			for _, change := range node.Blocks {
				if err = writeBlock(ctx, tx, node.Node, change.Index, change.Block, &touched); err != nil {
					return err
				}
				count++
			}
		}
		return blobs.CommitUses(touched)
	})
	return count, err
}
