package database

import (
	"context"
	"database/sql"
	"fmt"
	"slices"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

// InsertCommandOutputs writes rows while preserving each existing command row,
// recording blob uses before commit. The caller owns tx and rolls back on error.
func InsertCommandOutputs(ctx context.Context, tx *sql.Tx, blobs OwnerBlobs, rows []CommandOutput) error {
	touched := make([]core.BlobRef, 0)
	for _, row := range rows {
		var blob, missingBytes, missingReason, notStored, removed any
		switch output := row.Output.(type) {
		case *OutputStored:
			blob = output.Blob
			touched = append(touched, output.Blob)
			if output.Missing != nil {
				missingBytes = integer(output.Missing.Bytes)
				missingReason = output.Missing.Reason
			}
		case *OutputNotStored:
			notStored = output.Reason
		case *OutputRemoved:
			at, err := output.At.Millisecond()
			if err != nil {
				return err
			}
			removed = at
		}
		ended, err := row.Ended.Millisecond()
		if err != nil {
			return err
		}
		if err := execSQL(ctx, tx, "INSERT INTO command_outputs (command_id,ended_at,blob,missing_bytes,missing_reason,not_stored,removed_at) VALUES (?,?,?,?,?,?,?) ON CONFLICT (command_id) DO NOTHING", row.Command, ended, blob, missingBytes, missingReason, notStored, removed); err != nil {
			return err
		}
	}
	return blobs.CommitUses(ctx, touched)
}

// ReadCommandOutput reads a command's row; nil means it has no row.
func ReadCommandOutput(ctx context.Context, tx *sql.Tx, command core.CommandID) (*CommandOutput, error) {
	return queryRecord(ctx, tx, "command_outputs", "SELECT * FROM command_outputs WHERE command_id = ?", commandOutputRow, command)
}

// CommandOutputRows reads the commands' rows for a Fork to copy.
func CommandOutputRows(ctx context.Context, tx *sql.Tx, commands []core.CommandID) ([]CommandOutput, error) {
	rows := make([]CommandOutput, 0)
	for _, command := range commands {
		row, err := ReadCommandOutput(ctx, tx, command)
		if err != nil {
			return nil, err
		}
		if row != nil {
			rows = append(rows, *row)
		}
	}
	return rows, nil
}

// CommandsOf returns the commands named by blocks' shell calls, each once.
func CommandsOf(blocks []core.Block) []core.CommandID {
	commands := make([]core.CommandID, 0)
	for _, block := range blocks {
		if call, ok := block.(*core.ToolCallBlock); ok {
			if view, ok := call.View.(*core.ShellView); ok && !slices.Contains(commands, view.CommandID) {
				commands = append(commands, view.CommandID)
			}
		}
	}
	return commands
}

// HasExpiredOutputs reports whether stored output ended before expired.
func HasExpiredOutputs(ctx context.Context, tx *sql.Tx, expired core.Timestamp) (bool, error) {
	at, err := expired.Millisecond()
	if err != nil {
		return false, err
	}
	var exists bool
	err = tx.QueryRowContext(ctx, "SELECT EXISTS (SELECT 1 FROM command_outputs WHERE blob IS NOT NULL AND ended_at < ?)", at).Scan(&exists)
	return exists, err
}

// RemoveExpiredOutputs marks expired outputs removed at now and records released
// blobs' uses. The caller owns tx and rolls back on error. It returns the row count.
func RemoveExpiredOutputs(ctx context.Context, tx *sql.Tx, blobs OwnerBlobs, expired, now core.Timestamp) (int, error) {
	before, err := expired.Millisecond()
	if err != nil {
		return 0, err
	}
	at, err := now.Millisecond()
	if err != nil {
		return 0, err
	}
	released, err := queryRecords(ctx, tx, "command_outputs", "SELECT blob FROM command_outputs WHERE blob IS NOT NULL AND ended_at < ?", func(r *storedRow) core.BlobRef { return checked(r, "blob", core.ParseBlobRef) }, before)
	if err != nil {
		return 0, err
	}
	result, err := tx.ExecContext(ctx, "UPDATE command_outputs SET blob=NULL,missing_bytes=NULL,missing_reason=NULL,removed_at=? WHERE blob IS NOT NULL AND ended_at < ?", at, before)
	if err != nil {
		return 0, err
	}
	count, err := result.RowsAffected()
	if err != nil {
		return 0, err
	}
	return int(count), blobs.CommitUses(ctx, released)
}

// CommandOutputReferences returns the blobs held by command-output rows.
func CommandOutputReferences(ctx context.Context, tx *sql.Tx) ([]core.BlobRef, error) {
	return queryRecords(ctx, tx, "command_outputs", "SELECT blob FROM command_outputs WHERE blob IS NOT NULL", func(r *storedRow) core.BlobRef { return checked(r, "blob", core.ParseBlobRef) })
}

func commandOutputRow(r *storedRow) CommandOutput {
	row := CommandOutput{Command: checked(r, "command_id", core.ParseCommandID), Ended: r.instant("ended_at")}
	blob, reason, removed := r.values["blob"], r.values["not_stored"], r.values["removed_at"]
	switch {
	case blob != nil && reason == nil && removed == nil:
		output := &OutputStored{Blob: checked(r, "blob", core.ParseBlobRef)}
		if r.values["missing_bytes"] != nil && r.values["missing_reason"] != nil {
			output.Missing = &host.Missing{Bytes: r.count("missing_bytes"), Reason: r.text("missing_reason")}
		} else if r.values["missing_bytes"] != nil || r.values["missing_reason"] != nil {
			r.bad("missing_reason", fmt.Errorf("missing bytes come with their reason"))
		}
		row.Output = output
	case blob == nil && reason != nil && removed == nil:
		row.Output = &OutputNotStored{Reason: r.text("not_stored")}
	case blob == nil && reason == nil && removed != nil:
		row.Output = &OutputRemoved{At: r.instant("removed_at")}
	default:
		r.bad("blob", fmt.Errorf("a row is stored, not stored or removed"))
	}
	return row
}
