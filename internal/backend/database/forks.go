package database

import (
	"context"
	"database/sql"
	"fmt"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// ForkOperation returns the creation attempt that reserved `id`, in whichever case it is
// spelled.
func (c *ControlService) ForkOperation(ctx context.Context, id webapi.ConversationID) (*ForkOperation, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (*ForkOperation, error) {
		return forkByID(ctx, tx, id)
	})
}

// ReserveFork reserves `operation`'s destination: the attempt that already holds
// it when it is this one, none when another attempt or a conversation
// holds the id.
func (c *ControlService) ReserveFork(ctx context.Context, operation ForkOperation) (*ForkOperation, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (*ForkOperation, error) {
		existing, err := forkByID(ctx, tx, operation.ID)
		if err != nil {
			return nil, err
		}
		if existing != nil {
			if existing.SameAttempt(operation.Owner, operation.Source, operation.Block) {
				return existing, nil
			}
			return nil, nil
		}
		record, err := ConversationByID(ctx, tx, operation.ID)
		if err != nil || record != nil {
			return nil, err
		}
		text, err := encoded(operation.Metadata)
		if err != nil {
			return nil, err
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
		return &operation, err
	})
}

// PendingForks returns the attempts whose destination is not published.
func (c *ControlService) PendingForks(ctx context.Context) ([]ForkOperation, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) ([]ForkOperation, error) {
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
func (c *ControlService) PublishFork(ctx context.Context, id webapi.ConversationID) (ConversationRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (ConversationRecord, error) {
		o, err := forkByID(ctx, tx, id)
		if err != nil {
			return ConversationRecord{}, err
		}
		if o == nil {
			return ConversationRecord{}, &Error{
				Kind:   Corrupt,
				Table:  "conversation_fork_operations",
				Column: "id",
				Reason: fmt.Sprintf("no Fork reserved %s", id),
			}
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
		r, err := ConversationByID(ctx, tx, o.ID)
		if err != nil {
			return ConversationRecord{}, err
		}
		if r == nil || r.Owner != o.Owner {
			return ConversationRecord{}, &Error{
				Kind:   Corrupt,
				Table:  "conversations",
				Column: "user_id",
				Reason: fmt.Sprintf("the Fork destination %s belongs to another user", id),
			}
		}
		return *r, nil
	})
}

func forkRow(r *storedRow) ForkOperation {
	return ForkOperation{
		ID:       checked(r, "id", webapi.ParseConversationID),
		Owner:    checked(r, "user_id", webapi.ParseUserID),
		Source:   checked(r, "source_id", webapi.ParseConversationID),
		Block:    checked(r, "block_id", core.ParseBlockID),
		Metadata: storedJSON(r, "metadata", DecodeForkMetadata),
	}
}

func forkByID(ctx context.Context, tx *sql.Tx, id webapi.ConversationID) (*ForkOperation, error) {
	return queryRecord(
		ctx,
		tx,
		"conversation_fork_operations",
		"SELECT * FROM conversation_fork_operations WHERE id = ?",
		forkRow,
		id,
	)
}
