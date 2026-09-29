package storage

import (
	"context"
	"database/sql"
	"encoding/json/v2"
	"errors"
	"fmt"
	"strings"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type ForkOperation struct {
	ID       webapi.ConversationID
	Owner    webapi.UserID
	Source   webapi.ConversationID
	Block    core.BlockID
	Metadata ForkMetadata
}

//demi:wire
type ForkMetadata struct {
	Title         string                    `json:"title" check:"chars=1.."`
	Target        webapi.ConversationTarget `json:"target" check:"func=webapi.ValidateConversationTarget"`
	Model         *core.ModelSelection      `json:"model" check:"nullable,func=core.Validate"`
	CreatedAt     core.Timestamp            `json:"createdAt" check:"func=core.Validate"`
	AttachedHosts []AttachedHostRecord      `json:"attachedHosts"`
}

func (f ForkOperation) SameAttempt(owner webapi.UserID, source webapi.ConversationID, block core.BlockID) bool {
	return f.Owner == owner && strings.EqualFold(f.Source.String(), source.String()) && f.Block == block
}

const forkColumns = "id,user_id,source_id,block_id,metadata"

func scanFork(row scanner) (*ForkOperation, error) {
	var f ForkOperation
	var id, user, source, block, metadata string
	err := row.Scan(&id, &user, &source, &block, &metadata)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	f.ID, err = webapi.ParseConversationID(id)
	if err != nil {
		return nil, corrupt("conversation_fork_operations", "id", err)
	}
	f.Owner, err = webapi.ParseUserID(user)
	if err != nil {
		return nil, corrupt("conversation_fork_operations", "user_id", err)
	}
	f.Source, err = webapi.ParseConversationID(source)
	if err != nil {
		return nil, corrupt("conversation_fork_operations", "source_id", err)
	}
	f.Block, err = core.ParseBlockID(block)
	if err != nil {
		return nil, corrupt("conversation_fork_operations", "block_id", err)
	}
	if err = json.Unmarshal([]byte(metadata), &f.Metadata); err != nil {
		return nil, corrupt("conversation_fork_operations", "metadata", err)
	}
	return &f, nil
}
func (c *Control) ForkOperation(ctx context.Context, id webapi.ConversationID) (*ForkOperation, error) {
	return scanFork(c.db.QueryRowContext(ctx, "SELECT "+forkColumns+" FROM conversation_fork_operations WHERE id=?", id.String()))
}
func (c *Control) ReserveFork(ctx context.Context, operation ForkOperation) (*ForkOperation, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer tx.Rollback()
	existing, err := scanFork(tx.QueryRowContext(ctx, "SELECT "+forkColumns+" FROM conversation_fork_operations WHERE id=?", operation.ID.String()))
	if err != nil {
		return nil, err
	}
	if existing != nil {
		if existing.SameAttempt(operation.Owner, operation.Source, operation.Block) {
			return existing, nil
		}
		return nil, nil
	}
	var exists bool
	if err = tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?)", operation.ID.String()).Scan(&exists); err != nil {
		return nil, sqliteError(err)
	}
	if exists {
		return nil, nil
	}
	metadata, err := json.Marshal(operation.Metadata)
	if err != nil {
		return nil, err
	}
	_, err = tx.ExecContext(ctx, "INSERT INTO conversation_fork_operations("+forkColumns+") VALUES (?,?,?,?,?)", operation.ID.String(), operation.Owner.String(), operation.Source.String(), operation.Block.String(), string(metadata))
	if err != nil {
		return nil, sqliteError(err)
	}
	return &operation, sqliteError(tx.Commit())
}
func (c *Control) PendingForks(ctx context.Context) ([]ForkOperation, error) {
	rows, err := c.db.QueryContext(ctx, "SELECT f.id,f.user_id,f.source_id,f.block_id,f.metadata FROM conversation_fork_operations f LEFT JOIN conversations c ON c.id=f.id WHERE c.id IS NULL ORDER BY f.rowid")
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	operations := []ForkOperation{}
	for rows.Next() {
		operation, err := scanFork(rows)
		if err != nil {
			return nil, err
		}
		operations = append(operations, *operation)
	}
	return operations, sqliteError(rows.Err())
}
func (c *Control) PublishFork(ctx context.Context, id webapi.ConversationID) (*ConversationRecord, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer tx.Rollback()
	operation, err := scanFork(tx.QueryRowContext(ctx, "SELECT "+forkColumns+" FROM conversation_fork_operations WHERE id=?", id.String()))
	if err != nil {
		return nil, err
	}
	if operation == nil {
		return nil, &CorruptError{"conversation_fork_operations", "id", fmt.Sprintf("no Fork reserved %s", id.String())}
	}
	meta := operation.Metadata
	kind, device, path, workspace := targetColumns(meta.Target)
	var model *string
	if meta.Model != nil {
		data, err := json.Marshal(meta.Model)
		if err != nil {
			return nil, err
		}
		text := string(data)
		model = &text
	}
	result, err := tx.ExecContext(ctx, `INSERT INTO conversations(id,user_id,title,title_origin,archived,pinned,sort_order,read_revision,target_kind,target_device_id,target_path,target_workspace_id,context_version,model,user_messages,titled_messages,created_at,updated_at,live_at)
 VALUES (?1,?2,?3,'user',0,0,(SELECT COALESCE(MIN(sort_order),0)-1 FROM conversations WHERE user_id=?2),0,?4,?5,?6,?7,0,?8,0,0,?9,?9,?9) ON CONFLICT(id) DO NOTHING`, operation.ID.String(), operation.Owner.String(), meta.Title, kind, device, path, workspace, model, meta.CreatedAt.Millisecond())
	if err != nil {
		return nil, sqliteError(err)
	}
	count, err := result.RowsAffected()
	if err != nil {
		return nil, sqliteError(err)
	}
	if count == 1 {
		for _, host := range meta.AttachedHosts {
			var paired bool
			if err = tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?)", host.Device.String()).Scan(&paired); err != nil {
				return nil, sqliteError(err)
			}
			if paired {
				if _, err = insertAttachedHost(ctx, tx, operation.ID, host, meta.CreatedAt); err != nil {
					return nil, sqliteError(err)
				}
			}
		}
	}
	record, err := scanConversation(tx.QueryRowContext(ctx, "SELECT "+conversationColumns+" FROM conversations WHERE id=?", operation.ID.String()))
	if err != nil {
		return nil, err
	}
	if record == nil || record.Owner != operation.Owner {
		return nil, &CorruptError{"conversations", "user_id", fmt.Sprintf("the Fork destination %s belongs to another user", id.String())}
	}
	return record, sqliteError(tx.Commit())
}
