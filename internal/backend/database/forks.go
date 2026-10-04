package database

import (
	"context"
	"database/sql"
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// ForkOperation returns the creation attempt that reserved `id`, in whichever case it is
// spelled.
func (c *ControlService) ForkOperation(
	ctx context.Context,
	id webapiproto.ConversationID,
) (ForkOperation, bool, error) {
	var found bool
	record, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (ForkOperation, error) {
		r, ok, err := forkByID(ctx, tx, id)
		found = ok
		return r, err
	})
	return record, found && err == nil, err
}

// ReserveFork reserves `operation`'s destination: the attempt that already holds
// it when it is this one; ErrForkTaken when another attempt or a conversation
// holds the id.
func (c *ControlService) ReserveFork(ctx context.Context, operation ForkOperation) (ForkOperation, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (ForkOperation, error) {
		existing, found, err := forkByID(ctx, tx, operation.ID)
		if err != nil {
			return ForkOperation{}, err
		}
		if found {
			if existing.SameAttempt(operation.Owner, operation.Source, operation.Block) {
				return existing, nil
			}
			return ForkOperation{}, ErrForkTaken
		}
		_, taken, err := ConversationByID(ctx, tx, operation.ID)
		if err != nil {
			return ForkOperation{}, err
		}
		if taken {
			return ForkOperation{}, ErrForkTaken
		}
		text, err := encoded(operation.Metadata)
		if err != nil {
			return ForkOperation{}, err
		}
		err = execSQL(
			ctx,
			tx,
			"INSERT INTO conversation_fork_operations (id,user_id,source_id,block_id,metadata) VALUES (?,?,?,?,?)",
			operation.ID,
			operation.Owner,
			operation.Source,
			operation.Block,
			text,
		)
		return operation, err
	})
}

// PendingForks returns the attempts whose destination is not published.
func (c *ControlService) PendingForks(ctx context.Context) ([]ForkOperation, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) ([]ForkOperation, error) {
		return queryRecords(
			ctx,
			tx,
			"conversation_fork_operations",
			`SELECT f.*
FROM conversation_fork_operations f
LEFT JOIN conversations c ON c.id=f.id
WHERE c.id IS NULL
ORDER BY f.rowid`,
			forkRow,
		)
	})
}

// PublishFork publishes the destination `id` reserved: its conversation first in
// the owner's sidebar, with the title, target, model and attached hosts
// of the attempt, the title the user's. A destination published already
// is answered as it is. An attached host whose device is gone since is
// left out, as its revocation left the source.
func (c *ControlService) PublishFork(ctx context.Context, id webapiproto.ConversationID) (ConversationRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ types.Timestamp) (ConversationRecord, error) {
		o, found, err := forkByID(ctx, tx, id)
		if err != nil {
			return ConversationRecord{}, err
		}
		if !found {
			return ConversationRecord{}, CorruptValue(
				"conversation_fork_operations",
				"id",
				fmt.Errorf("no Fork reserved %s", id),
			)
		}
		m := o.Metadata
		inserted, err := InsertConversation(
			ctx,
			tx,
			NewConversation{
				ID:     o.ID,
				Owner:  o.Owner,
				Title:  m.Title,
				Origin: TitleUser,
				Target: m.Target,
				Model:  m.Model,
				At:     m.CreatedAt,
			},
		)
		if err != nil {
			return ConversationRecord{}, err
		}
		if inserted == 1 {
			for _, h := range m.AttachedHosts {
				var exists bool
				if err := tx.QueryRowContext(ctx, "SELECT EXISTS (SELECT 1 FROM devices WHERE id = ?)", h.Device).
					Scan(&exists); err != nil {
					return ConversationRecord{}, err
				}
				if exists {
					if _, err := InsertAttachedHost(ctx, tx, o.ID, h, m.CreatedAt); err != nil {
						return ConversationRecord{}, err
					}
				}
			}
		}
		r, ok, err := ConversationByID(ctx, tx, o.ID)
		if err != nil {
			return ConversationRecord{}, err
		}
		if !ok || r.Owner != o.Owner {
			return ConversationRecord{}, CorruptValue(
				"conversations",
				"user_id",
				fmt.Errorf("the Fork destination %s belongs to another user", id),
			)
		}
		return r, nil
	})
}

func forkRow(r *storedRow) ForkOperation {
	return ForkOperation{
		ID:       checked(r, "id", webapiproto.ParseConversationID),
		Owner:    checked(r, "user_id", webapiproto.ParseUserID),
		Source:   checked(r, "source_id", webapiproto.ParseConversationID),
		Block:    checked(r, "block_id", types.ParseBlockID),
		Metadata: storedJSON(r, "metadata", DecodeForkMetadata),
	}
}

func forkByID(ctx context.Context, tx *sql.Tx, id webapiproto.ConversationID) (ForkOperation, bool, error) {
	return queryRecord(
		ctx,
		tx,
		"conversation_fork_operations",
		"SELECT * FROM conversation_fork_operations WHERE id = ?",
		forkRow,
		id,
	)
}

// ErrForkTaken means another attempt or a conversation holds the ID.
var ErrForkTaken = errors.New("another attempt or a conversation holds the id")
