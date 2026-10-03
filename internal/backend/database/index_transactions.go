package database

import (
	"context"
	"database/sql"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

const insertConversationSQL = `INSERT INTO conversations (
    id,
    user_id,
    title,
    title_origin,
    archived,
    pinned,
    sort_order,
    read_revision,
    target_kind,
    target_device_id,
    target_path,
    target_workspace_id,
    context_version,
    model,
    user_messages,
    titled_messages,
    created_at,
    updated_at,
    live_at
)
VALUES (
    ?1,
    ?2,
    ?3,
    ?4,
    0,
    0,
    (SELECT COALESCE(MIN(sort_order),0)-1 FROM conversations WHERE user_id=?2),
    0,
    ?5,
    ?6,
    ?7,
    ?8,
    0,
    ?9,
    0,
    0,
    ?10,
    ?10,
    ?10
)
ON CONFLICT (id) DO NOTHING`

// InsertConversation inserts new first in its owner's sidebar, unarchived and
// unpinned, unless its ID exists in any spelling. It returns the inserted count.
func InsertConversation(ctx context.Context, tx *sql.Tx, record NewConversation) (int, error) {
	target := ColumnsForTarget(record.Target)
	var model *string
	if record.Model != nil {
		text, err := encoded(record.Model)
		if err != nil {
			return 0, err
		}
		model = &text
	}
	at, err := record.At.Millisecond()
	if err != nil {
		return 0, err
	}
	origin := "placeholder"
	if record.Origin == TitleUser {
		origin = "user"
	}
	result, err := tx.ExecContext(
		ctx,
		insertConversationSQL,
		record.ID,
		record.Owner,
		record.Title,
		origin,
		target.Kind,
		target.Device,
		target.Path,
		target.Workspace,
		model,
		at,
	)
	if err != nil {
		return 0, err
	}
	count, err := result.RowsAffected()
	return int(count), err
}

// ConversationByID reads a conversation in any spelling of its ID.
func ConversationByID(ctx context.Context, tx *sql.Tx, id webapi.ConversationID) (*ConversationRecord, error) {
	return queryRecord(
		ctx,
		tx,
		"conversations",
		"SELECT "+conversationColumns+" FROM conversations WHERE id = ?",
		conversationRow,
		id,
	)
}

// InsertAttachedHost attaches a new device under the first free name: name,
// name-2, name-3, and so on; an empty name becomes its ID. An existing device
// keeps its row and returns false. The caller owns the transaction.
func InsertAttachedHost(
	ctx context.Context,
	tx *sql.Tx,
	conversation webapi.ConversationID,
	host AttachedHostRecord,
	now core.Timestamp,
) (bool, error) {
	base := strings.TrimSpace(host.Name)
	if base == "" {
		base = string(host.Device)
	}
	names, err := queryRecords(
		ctx,
		tx,
		"conversation_hosts",
		"SELECT name FROM conversation_hosts WHERE conversation_id = ?",
		func(r *storedRow) string { return r.text("name") },
		conversation,
	)
	if err != nil {
		return false, err
	}
	taken := map[string]bool{}
	for _, name := range names {
		taken[name] = true
	}
	candidate := base
	for suffix := 2; taken[candidate]; suffix++ {
		candidate = fmt.Sprintf("%s-%d", base, suffix)
	}
	at, err := now.Millisecond()
	if err != nil {
		return false, err
	}
	return affected(
		ctx,
		tx,
		`INSERT INTO conversation_hosts (conversation_id,device_id,name,cwd,attached_at)
VALUES (?,?,?,?,?)
ON CONFLICT (conversation_id,device_id) DO NOTHING`,
		conversation,
		host.Device,
		candidate,
		host.CWD,
		at,
	)
}
