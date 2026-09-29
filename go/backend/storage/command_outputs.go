package storage

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"math"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/runnerproto"
)

type OutputRow interface{ outputRow() }
type OutputStored struct {
	Blob    core.BlobRef
	Missing *MissingOutput
}

func (OutputStored) outputRow() {}

type MissingOutput struct {
	Bytes  uint64
	Reason string
}
type OutputNotStored string

func (OutputNotStored) outputRow() {}

type OutputRemoved struct{ At core.Timestamp }

func (OutputRemoved) outputRow() {}

type CommandOutput struct {
	Command core.CommandID
	Ended   core.Timestamp
	Output  OutputRow
}

const outputColumns = "command_id,ended_at,blob,missing_bytes,missing_reason,not_stored,removed_at"

func scanOutput(row scanner) (*CommandOutput, error) {
	var r CommandOutput
	var command string
	var ended int64
	var blob, reason, notStored *string
	var missing *uint64
	var removed *int64
	err := row.Scan(&command, &ended, &blob, &missing, &reason, &notStored, &removed)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	r.Command, err = core.ParseCommandID(command)
	if err != nil {
		return nil, corrupt("command_outputs", "command_id", err)
	}
	r.Ended, err = instant("command_outputs", "ended_at", ended)
	if err != nil {
		return nil, err
	}
	switch {
	case blob != nil && notStored == nil && removed == nil:
		ref, err := core.ParseBlobRef(*blob)
		if err != nil {
			return nil, corrupt("command_outputs", "blob", err)
		}
		value := OutputStored{Blob: ref}
		if (missing == nil) != (reason == nil) {
			return nil, &CorruptError{"command_outputs", "missing_reason", "missing bytes come with their reason"}
		}
		if missing != nil {
			value.Missing = &MissingOutput{*missing, *reason}
		}
		r.Output = value
	case blob == nil && notStored != nil && removed == nil:
		r.Output = OutputNotStored(*notStored)
	case blob == nil && notStored == nil && removed != nil:
		at, err := instant("command_outputs", "removed_at", *removed)
		if err != nil {
			return nil, err
		}
		r.Output = OutputRemoved{at}
	default:
		return nil, &CorruptError{"command_outputs", "blob", "a row is stored, not stored or removed"}
	}
	return &r, nil
}
func (d *ConversationDB) InsertOutputs(ctx context.Context, blobs *UserBlobs, outputs []CommandOutput) error {
	return d.transaction(ctx, func(tx *sql.Tx) error {
		touched := []core.BlobRef{}
		for _, row := range outputs {
			var blob, reason, notStored *string
			var missing, removed *int64
			switch value := row.Output.(type) {
			case OutputStored:
				text := value.Blob.String()
				blob = &text
				touched = append(touched, value.Blob)
				if value.Missing != nil {
					count := int64(min(value.Missing.Bytes, math.MaxInt64))
					missing = &count
					reason = &value.Missing.Reason
				}
			case OutputNotStored:
				text := string(value)
				notStored = &text
			case OutputRemoved:
				ms := value.At.Millisecond()
				removed = &ms
			}
			_, err := tx.ExecContext(ctx, "INSERT INTO command_outputs("+outputColumns+") VALUES (?,?,?,?,?,?,?) ON CONFLICT(command_id) DO NOTHING", row.Command.String(), row.Ended.Millisecond(), blob, missing, reason, notStored, removed)
			if err != nil {
				return sqliteError(err)
			}
		}
		return blobs.CommitUses(touched)
	})
}
func (d *ConversationDB) CommandOutput(ctx context.Context, command core.CommandID) (*CommandOutput, error) {
	var output *CommandOutput
	_, err := d.read(ctx, func(tx *sql.Tx) error {
		var err error
		output, err = scanOutput(tx.QueryRowContext(ctx, "SELECT "+outputColumns+" FROM command_outputs WHERE command_id=?", command.String()))
		return err
	})
	return output, err
}
func (d *ConversationDB) OutputRows(ctx context.Context, commands []core.CommandID) ([]CommandOutput, error) {
	outputs := []CommandOutput{}
	_, err := d.read(ctx, func(tx *sql.Tx) error {
		for _, command := range commands {
			row, err := scanOutput(tx.QueryRowContext(ctx, "SELECT "+outputColumns+" FROM command_outputs WHERE command_id=?", command.String()))
			if err != nil {
				return err
			}
			if row != nil {
				outputs = append(outputs, *row)
			}
		}
		return nil
	})
	return outputs, err
}
func CommandsOf(blocks []core.Block) []core.CommandID {
	commands := []core.CommandID{}
	seen := map[core.CommandID]bool{}
	for _, block := range blocks {
		b, ok := block.(core.BlockToolCall)
		if !ok || b.View == nil {
			continue
		}
		view, ok := (*b.View).(core.ToolViewShell)
		if ok && !seen[view.CommandID] {
			commands = append(commands, view.CommandID)
			seen[view.CommandID] = true
		}
	}
	return commands
}
func (d *ConversationDB) HasExpiredOutputs(ctx context.Context, expired core.Timestamp) (bool, error) {
	var exists bool
	_, err := d.read(ctx, func(tx *sql.Tx) error {
		return sqliteError(tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM command_outputs WHERE blob IS NOT NULL AND ended_at<?)", expired.Millisecond()).Scan(&exists))
	})
	return exists, err
}
func (d *ConversationDB) RemoveExpiredOutputs(ctx context.Context, blobs *UserBlobs, expired, now core.Timestamp) (int64, error) {
	var count int64
	err := d.transaction(ctx, func(tx *sql.Tx) error {
		released, err := queryBlobs(ctx, tx, "SELECT blob FROM command_outputs WHERE blob IS NOT NULL AND ended_at<?", expired.Millisecond())
		if err != nil {
			return err
		}
		result, err := tx.ExecContext(ctx, "UPDATE command_outputs SET blob=NULL,missing_bytes=NULL,missing_reason=NULL,removed_at=? WHERE blob IS NOT NULL AND ended_at<?", now.Millisecond(), expired.Millisecond())
		if err != nil {
			return sqliteError(err)
		}
		count, err = result.RowsAffected()
		if err != nil {
			return sqliteError(err)
		}
		return blobs.CommitUses(released)
	})
	return count, err
}

// StoredCommandOutput exposes the persisted runner records without inventing an
// agent WholeOutput type. G7d can wrap these with its output view.
type StoredCommandOutput struct {
	Row     CommandOutput
	Records []runnerproto.KeptRecord
}

func (s *TreeStore) ReadCommandOutput(ctx context.Context, command core.CommandID) (*StoredCommandOutput, error) {
	row, err := s.DB.CommandOutput(ctx, command)
	if err != nil || row == nil {
		return nil, err
	}
	result := &StoredCommandOutput{Row: *row}
	stored, ok := row.Output.(OutputStored)
	if !ok {
		return result, nil
	}
	data, err := s.Blobs.Get(ctx, stored.Blob)
	if err != nil {
		return nil, err
	}
	if data == nil {
		return nil, fmt.Errorf("the blob %s of the output of %s is missing", stored.Blob.String(), command.String())
	}
	result.Records, err = runnerproto.DecodeRecords(data)
	if err != nil {
		return nil, fmt.Errorf("the output of %s does not decode: %w", command.String(), err)
	}
	return result, nil
}
