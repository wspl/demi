package database

import (
	"context"
	"database/sql"
	"math"
	"slices"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/types"
)

// OwnerBlobs is the owner's namespace and blob-use record needed by commits.
// Implementations support concurrent calls and never call back into the database.
type OwnerBlobs interface {
	// Media is the namespace as a session reaches it through its tree store.
	Media() store.Blobs
	// CommitUses records references written or removed inside a transaction before
	// commit. A blob being deleted refuses the transaction. This operation performs
	// no network IO; cancellation must not abandon an admitted commit.
	CommitUses(ctx context.Context, blobs []types.BlobRef) error
}

// RetiredNode holds changed blocks in one node after applying the agent's retirement rule.
type RetiredNode struct {
	Node   types.NodeID
	Blocks []transcript.RetiredBlock
}

// BlobRows derives the index rows of block in reference order.
func BlobRows(block types.Block) []BlobRefRow {
	rows := make([]BlobRefRow, 0)
	for part, ref := range store.BlockReferences(block) {
		rows = append(rows, BlobRefRow{Part: part, Blob: ref.Blob, Holder: ref.Holder, At: block.CreatedAt()})
	}
	return rows
}

// HolderName returns the holder column's stored spelling.
func HolderName(holder store.Holder) string {
	switch holder {
	case store.Message:
		return "message"
	case store.ToolResult:
		return "tool_result"
	case store.EditCopy:
		return "edit_copy"
	}
	return ""
}

// WriteBlock replaces a block and its index rows, adding old and new blobs to touched.
func WriteBlock(
	ctx context.Context,
	tx *sql.Tx,
	node types.NodeID,
	index int,
	block types.Block,
	touched *[]types.BlobRef,
) error {
	document, err := encoded(block)
	if err != nil {
		return err
	}
	if err := execSQL(
		ctx,
		tx,
		"INSERT INTO blocks (node_id,idx,block) VALUES (?,?,?) ON CONFLICT (node_id,idx) DO UPDATE SET block=excluded.block",
		node,
		index,
		document,
	); err != nil {
		return err
	}
	if err := removedBlobs(
		ctx,
		tx,
		"DELETE FROM blob_refs WHERE node_id = ? AND idx = ? RETURNING blob",
		node,
		index,
		touched,
	); err != nil {
		return err
	}
	for _, row := range BlobRows(block) {
		at, err := row.At.Millisecond()
		if err != nil {
			return err
		}
		if err := execSQL(
			ctx,
			tx,
			"INSERT INTO blob_refs (node_id,idx,part,blob,holder,at) VALUES (?,?,?,?,?,?)",
			node,
			index,
			row.Part,
			row.Blob,
			HolderName(row.Holder),
			at,
		); err != nil {
			return err
		}
		*touched = append(*touched, row.Blob)
	}
	return nil
}

// TruncateBlocks removes blocks and index rows from index on, adding removed blobs to touched.
func TruncateBlocks(ctx context.Context, tx *sql.Tx, node types.NodeID, index int, touched *[]types.BlobRef) error {
	if err := removedBlobs(
		ctx,
		tx,
		"DELETE FROM blob_refs WHERE node_id = ? AND idx >= ? RETURNING blob",
		node,
		index,
		touched,
	); err != nil {
		return err
	}
	return execSQL(ctx, tx, "DELETE FROM blocks WHERE node_id = ? AND idx >= ?", node, index)
}

// SubtreeBlobs returns index blobs referenced by node and its descendants.
func SubtreeBlobs(ctx context.Context, tx *sql.Tx, node types.NodeID) ([]types.BlobRef, error) {
	return queryRecords(
		ctx,
		tx,
		"blob_refs",
		`WITH RECURSIVE subtree (id) AS (
    SELECT ?
    UNION
    SELECT nodes.id
    FROM nodes
    JOIN subtree ON nodes.parent_id=subtree.id
)
SELECT blob
FROM blob_refs
WHERE node_id IN subtree`,
		func(r *storedRow) types.BlobRef {
			return checked(r, "blob", types.ParseBlobRef)
		},
		node,
	)
}

// Retirable reads indexed nodes with expired tool media and applies the agent's rule.
func Retirable(ctx context.Context, tx *sql.Tx, retirement transcript.Retirement) ([]RetiredNode, error) {
	at, err := later(retirement.Now, -transcript.Kept)
	expired := int64(math.MinInt64)
	if err == nil {
		expired, err = at.Millisecond()
		if err != nil {
			return nil, err
		}
	}
	type nodeCount struct {
		id    types.NodeID
		count int64
	}
	nodes, err := queryRecords(
		ctx,
		tx,
		"nodes",
		`SELECT id,block_count
FROM nodes
WHERE id IN (SELECT node_id FROM blob_refs WHERE holder = ? AND at < ?)
ORDER BY id`,
		func(r *storedRow) nodeCount {
			return nodeCount{id: checked(r, "id", types.ParseNodeID), count: r.integer("block_count")}
		},
		HolderName(store.ToolResult),
		expired,
	)
	if err != nil {
		return nil, err
	}
	retired := make([]RetiredNode, 0)
	for _, node := range nodes {
		blocks, err := blocksOf(ctx, tx, node.id, node.count)
		if err != nil {
			return nil, err
		}
		changed := transcript.Retire(blocks, retirement)
		if len(changed) > 0 {
			retired = append(retired, RetiredNode{Node: node.id, Blocks: changed})
		}
	}
	return retired, nil
}

// RetireMedia replaces expired tool media and its index rows, recording blob uses
// before commit. It returns the changed block count without advancing revisions.
// The caller owns tx and must roll it back on any error, including a store refusal.
func RetireMedia(ctx context.Context, tx *sql.Tx, blobs OwnerBlobs, retirement transcript.Retirement) (int, error) {
	retired, err := Retirable(ctx, tx, retirement)
	if err != nil {
		return 0, err
	}
	touched := make([]types.BlobRef, 0)
	changed := 0
	for _, node := range retired {
		for _, block := range node.Blocks {
			if err := WriteBlock(ctx, tx, node.Node, block.Index, block.Value, &touched); err != nil {
				return 0, err
			}
			changed++
		}
	}
	return changed, blobs.CommitUses(ctx, touched)
}

// References returns distinct blobs held by blocks, queued messages and command outputs.
func References(ctx context.Context, tx *sql.Tx) ([]types.BlobRef, error) {
	refs, err := CommandOutputReferences(ctx, tx)
	if err != nil {
		return nil, err
	}
	indexed, err := queryRecords(
		ctx,
		tx,
		"blob_refs",
		"SELECT DISTINCT blob FROM blob_refs",
		func(r *storedRow) types.BlobRef {
			return checked(r, "blob", types.ParseBlobRef)
		},
	)
	if err != nil {
		return nil, err
	}
	refs = append(refs, indexed...)
	states, err := queryRecords(
		ctx,
		tx,
		"nodes",
		"SELECT state FROM nodes",
		func(r *storedRow) store.CheckpointState {
			return storedJSON(r, "state", store.DecodeCheckpointState)
		},
	)
	if err != nil {
		return nil, err
	}
	for _, state := range states {
		for _, message := range state.Queue {
			refs = append(refs, store.ContentReferences(message.Content)...)
		}
	}
	slices.Sort(refs)
	return slices.Compact(refs), nil
}

func removedBlobs(
	ctx context.Context,
	tx *sql.Tx,
	query string,
	node types.NodeID,
	index int,
	touched *[]types.BlobRef,
) error {
	rows, err := queryRecords(
		ctx,
		tx,
		"blob_refs",
		query,
		func(r *storedRow) types.BlobRef {
			return checked(r, "blob", types.ParseBlobRef)
		},
		node,
		index,
	)
	if err != nil {
		return err
	}
	*touched = append(*touched, rows...)
	return nil
}
