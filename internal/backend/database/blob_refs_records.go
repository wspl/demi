package database

import (
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

// BlobRefRow describes one row of the index: a blob one block references.
type BlobRefRow struct {
	// The reference's place among the block's references.
	Part   int
	Blob   core.BlobRef
	Holder store.Holder
	// The block's time.
	At core.Timestamp
}
