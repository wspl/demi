package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"
	"database/sql"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
)

// OwnerBlobs is the owner's namespace and blob-use record needed by commits.
// Implementations support concurrent calls and never call back into the database.
type OwnerBlobs interface {
	// Media is the namespace as a session reaches it through its tree store.
	Media() store.BlobStore
	// CommitUses records references written or removed inside a transaction before
	// commit. A blob being deleted refuses the transaction. This operation performs
	// no network IO; cancellation must not abandon an admitted commit.
	CommitUses(ctx context.Context, blobs []core.BlobRef) error
}

// RetiredNode holds changed blocks in one node after applying the agent's retirement rule.
type RetiredNode struct {
	Node   core.NodeID
	Blocks []transcript.RetiredBlock
}

// BlobRows derives the index rows of block in reference order.
func BlobRows(block core.Block) []BlobRefRow { panic("not written: b-database") }

// HolderName returns the holder column's stored spelling.
func HolderName(holder store.Holder) string { panic("not written: b-database") }

// WriteBlock replaces a block and its index rows, adding old and new blobs to touched.
func WriteBlock(ctx context.Context, tx *sql.Tx, node core.NodeID, index int, block core.Block, touched *[]core.BlobRef) error {
	panic("not written: b-database")
}

// TruncateBlocks removes blocks and index rows from index on, adding removed blobs to touched.
func TruncateBlocks(ctx context.Context, tx *sql.Tx, node core.NodeID, index int, touched *[]core.BlobRef) error {
	panic("not written: b-database")
}

// SubtreeBlobs returns index blobs referenced by node and its descendants.
func SubtreeBlobs(ctx context.Context, tx *sql.Tx, node core.NodeID) ([]core.BlobRef, error) {
	panic("not written: b-database")
}

// Retirable reads indexed nodes with expired tool media and applies the agent's rule.
func Retirable(ctx context.Context, tx *sql.Tx, retirement transcript.Retirement) ([]RetiredNode, error) {
	panic("not written: b-database")
}

// RetireMedia replaces expired tool media and its index rows, recording blob uses
// before commit. It returns the changed block count without advancing revisions.
// The caller owns tx and must roll it back on any error, including a store refusal.
func RetireMedia(ctx context.Context, tx *sql.Tx, blobs OwnerBlobs, retirement transcript.Retirement) (int, error) {
	panic("not written: b-database")
}

// References returns distinct blobs held by blocks, queued messages and command outputs.
func References(ctx context.Context, tx *sql.Tx) ([]core.BlobRef, error) {
	panic("not written: b-database")
}
