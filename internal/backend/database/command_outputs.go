package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"
	"database/sql"

	"github.com/wspl/demi/internal/core"
)

// InsertCommandOutputs writes rows while preserving each existing command row,
// recording blob uses before commit. The caller owns tx and rolls back on error.
func InsertCommandOutputs(ctx context.Context, tx *sql.Tx, blobs OwnerBlobs, rows []CommandOutput) error {
	panic("not written: b-database")
}

// ReadCommandOutput reads a command's row; nil means it has no row.
func ReadCommandOutput(ctx context.Context, tx *sql.Tx, command core.CommandID) (*CommandOutput, error) {
	panic("not written: b-database")
}

// CommandOutputRows reads the commands' rows for a Fork to copy.
func CommandOutputRows(ctx context.Context, tx *sql.Tx, commands []core.CommandID) ([]CommandOutput, error) {
	panic("not written: b-database")
}

// CommandsOf returns the commands named by blocks' shell calls, each once.
func CommandsOf(blocks []core.Block) []core.CommandID { panic("not written: b-database") }

// HasExpiredOutputs reports whether stored output ended before expired.
func HasExpiredOutputs(ctx context.Context, tx *sql.Tx, expired core.Timestamp) (bool, error) {
	panic("not written: b-database")
}

// RemoveExpiredOutputs marks expired outputs removed at now and records released
// blobs' uses. The caller owns tx and rolls back on error. It returns the row count.
func RemoveExpiredOutputs(ctx context.Context, tx *sql.Tx, blobs OwnerBlobs, expired, now core.Timestamp) (int, error) {
	panic("not written: b-database")
}

// CommandOutputReferences returns the blobs held by command-output rows.
func CommandOutputReferences(ctx context.Context, tx *sql.Tx) ([]core.BlobRef, error) {
	panic("not written: b-database")
}
