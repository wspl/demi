package storage

import (
	"context"
	"database/sql"
	"encoding/json/v2"
	"errors"
	"math"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type ConversationRecord struct {
	ID                           webapi.ConversationID
	Owner                        webapi.UserID
	Title                        string
	Archived, Pinned             bool
	ReadRevision                 uint64
	Target                       webapi.ConversationTarget
	ContextVersion               uint64
	Model                        *core.ModelSelection
	UserMessages, TitledMessages uint64
	CreatedAt, UpdatedAt         core.Timestamp
	DraftRevision                uint64
}
type Creation struct {
	Kind   CreationKind
	Record *ConversationRecord
}
type CreationKind string

const (
	Created     CreationKind = "created"
	Existing    CreationKind = "existing"
	Unavailable CreationKind = "unavailable"
)
const conversationColumns = `id,user_id,title,archived,pinned,read_revision,target_kind,target_device_id,target_path,target_workspace_id,context_version,model,user_messages,titled_messages,created_at,updated_at,COALESCE((SELECT revision FROM conversation_drafts WHERE conversation_id=conversations.id),0)`

func scanConversation(row scanner) (*ConversationRecord, error) {
	var r ConversationRecord
	var id, owner, kind string
	var device, path, workspace, model *string
	var created, updated int64
	err := row.Scan(&id, &owner, &r.Title, &r.Archived, &r.Pinned, storedCount{"conversations", "read_revision", &r.ReadRevision}, &kind, &device, &path, &workspace, storedCount{"conversations", "context_version", &r.ContextVersion}, &model, storedCount{"conversations", "user_messages", &r.UserMessages}, storedCount{"conversations", "titled_messages", &r.TitledMessages}, &created, &updated, storedCount{"conversation_drafts", "revision", &r.DraftRevision})
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	r.ID, err = webapi.ParseConversationID(id)
	if err != nil {
		return nil, corrupt("conversations", "id", err)
	}
	r.Owner, err = webapi.ParseUserID(owner)
	if err != nil {
		return nil, corrupt("conversations", "user_id", err)
	}
	switch kind {
	case "cloud":
		r.Target = webapi.ConversationTargetCloud{Path: path}
	case "device":
		if device == nil || path == nil {
			return nil, &CorruptError{"conversations", "target_device_id", "missing device target"}
		}
		value, err := webapi.ParseDeviceID(*device)
		if err != nil {
			return nil, corrupt("conversations", "target_device_id", err)
		}
		r.Target = webapi.ConversationTargetDevice{DeviceID: value, Path: *path}
	case "workspace":
		if workspace == nil {
			return nil, &CorruptError{"conversations", "target_workspace_id", "missing workspace target"}
		}
		value, err := webapi.ParseWorkspaceID(*workspace)
		if err != nil {
			return nil, corrupt("conversations", "target_workspace_id", err)
		}
		r.Target = webapi.ConversationTargetWorkspace{WorkspaceID: value}
	default:
		return nil, &CorruptError{"conversations", "target_kind", "unknown target kind " + kind}
	}
	if err = webapi.ValidateConversationTarget(r.Target); err != nil {
		return nil, corrupt("conversations", "target_path", err)
	}
	if model != nil {
		value, err := core.Decode[core.ModelSelection]([]byte(*model))
		if err != nil {
			return nil, corrupt("conversations", "model", err)
		}
		r.Model = &value
	}
	r.CreatedAt, err = instant("conversations", "created_at", created)
	if err != nil {
		return nil, err
	}
	r.UpdatedAt, err = instant("conversations", "updated_at", updated)
	return &r, err
}
func (c *Control) CreateConversation(ctx context.Context, owner webapi.UserID, id webapi.ConversationID) (Creation, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return Creation{}, sqliteError(err)
	}
	defer tx.Rollback()
	var reserved bool
	if err = tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM conversation_fork_operations WHERE id=?)", id.String()).Scan(&reserved); err != nil {
		return Creation{}, sqliteError(err)
	}
	if reserved {
		return Creation{Kind: Unavailable}, nil
	}
	result, err := tx.ExecContext(ctx, `INSERT INTO conversations(id,user_id,title,title_origin,archived,pinned,sort_order,read_revision,target_kind,context_version,model,user_messages,titled_messages,created_at,updated_at,live_at)
 VALUES (?1,?2,'New conversation','placeholder',0,0,(SELECT COALESCE(MIN(sort_order),0)-1 FROM conversations WHERE user_id=?2),0,'cloud',0,NULL,0,0,?3,?3,?3) ON CONFLICT(id) DO NOTHING`, id.String(), owner.String(), c.clock.Now().Millisecond())
	if err != nil {
		return Creation{}, sqliteError(err)
	}
	count, err := result.RowsAffected()
	if err != nil {
		return Creation{}, sqliteError(err)
	}
	record, err := scanConversation(tx.QueryRowContext(ctx, "SELECT "+conversationColumns+" FROM conversations WHERE id=?", id.String()))
	if err != nil {
		return Creation{}, err
	}
	if record == nil {
		return Creation{}, &CorruptError{"conversations", "id", "a conversation just created or found is missing"}
	}
	if err = tx.Commit(); err != nil {
		return Creation{}, sqliteError(err)
	}
	if record.Owner != owner {
		return Creation{Kind: Unavailable}, nil
	}
	kind := Existing
	if count == 1 {
		kind = Created
	}
	return Creation{kind, record}, nil
}
func (c *Control) Conversation(ctx context.Context, id webapi.ConversationID) (*ConversationRecord, error) {
	return scanConversation(c.db.QueryRowContext(ctx, "SELECT "+conversationColumns+" FROM conversations WHERE id=?", id.String()))
}
func (c *Control) Conversations(ctx context.Context, owner webapi.UserID, archived bool) ([]ConversationRecord, error) {
	rows, err := c.db.QueryContext(ctx, "SELECT "+conversationColumns+" FROM conversations WHERE user_id=? AND archived=? ORDER BY pinned DESC,sort_order,id", owner.String(), archived)
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	records := []ConversationRecord{}
	for rows.Next() {
		r, err := scanConversation(rows)
		if err != nil {
			return nil, err
		}
		records = append(records, *r)
	}
	return records, sqliteError(rows.Err())
}
func (c *Control) ConversationOrder(ctx context.Context, owner webapi.UserID) ([]webapi.ConversationID, error) {
	rows, err := c.db.QueryContext(ctx, "SELECT id FROM conversations WHERE user_id=? ORDER BY archived,pinned DESC,sort_order,id", owner.String())
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	ids := []webapi.ConversationID{}
	for rows.Next() {
		var text string
		if err = rows.Scan(&text); err != nil {
			return nil, sqliteError(err)
		}
		id, err := webapi.ParseConversationID(text)
		if err != nil {
			return nil, corrupt("conversations", "id", err)
		}
		ids = append(ids, id)
	}
	return ids, sqliteError(rows.Err())
}
func (c *Control) MarkConversationRead(ctx context.Context, id webapi.ConversationID, revision uint64) error {
	_, err := c.db.ExecContext(ctx, "UPDATE conversations SET read_revision=MAX(read_revision,?) WHERE id=?", int64(min(revision, math.MaxInt64)), id.String())
	return sqliteError(err)
}
func (c *Control) CountUserMessage(ctx context.Context, id webapi.ConversationID) (uint64, error) {
	var count uint64
	err := c.db.QueryRowContext(ctx, "UPDATE conversations SET user_messages=user_messages+1 WHERE id=? RETURNING user_messages", id.String()).Scan(&count)
	return count, sqliteError(err)
}
func (c *Control) TitleFromFirstMessage(ctx context.Context, id webapi.ConversationID, title string) (bool, error) {
	result, err := c.db.ExecContext(ctx, "UPDATE conversations SET title=?,title_origin='message' WHERE id=? AND title_origin='placeholder'", title, id.String())
	if err != nil {
		return false, sqliteError(err)
	}
	count, err := result.RowsAffected()
	return count == 1, sqliteError(err)
}
func (c *Control) GeneratedTitle(ctx context.Context, id webapi.ConversationID, title, from string, seen uint64) (bool, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return false, sqliteError(err)
	}
	defer tx.Rollback()
	result, err := tx.ExecContext(ctx, "UPDATE conversations SET title=?,title_origin='generated' WHERE id=? AND title=?", title, id.String(), from)
	if err != nil {
		return false, sqliteError(err)
	}
	count, err := result.RowsAffected()
	if err != nil {
		return false, sqliteError(err)
	}
	if _, err = tx.ExecContext(ctx, "UPDATE conversations SET titled_messages=MAX(titled_messages,?) WHERE id=?", seen, id.String()); err != nil {
		return false, sqliteError(err)
	}
	return count == 1, sqliteError(tx.Commit())
}
func (c *Control) MarkLive(ctx context.Context, id webapi.ConversationID, at core.Timestamp) error {
	_, err := c.db.ExecContext(ctx, "UPDATE conversations SET live_at=MAX(live_at,?) WHERE id=?", at.Millisecond(), id.String())
	return sqliteError(err)
}
func (c *Control) LiveAt(ctx context.Context, id webapi.ConversationID) (*core.Timestamp, error) {
	var ms int64
	err := c.db.QueryRowContext(ctx, "SELECT live_at FROM conversations WHERE id=?", id.String()).Scan(&ms)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	at, err := instant("conversations", "live_at", ms)
	return &at, err
}
func (c *Control) TouchConversation(ctx context.Context, id webapi.ConversationID) error {
	_, err := c.db.ExecContext(ctx, "UPDATE conversations SET updated_at=? WHERE id=?", c.clock.Now().Millisecond(), id.String())
	return sqliteError(err)
}

type RecordChange interface{ recordChange() }
type ChangeArchived bool

func (ChangeArchived) recordChange() {}

type ChangeTitle string

func (ChangeTitle) recordChange() {}

type ChangePinned bool

func (ChangePinned) recordChange() {}

type ChangeModel struct{ Model core.ModelSelection }

func (ChangeModel) recordChange() {}

type ChangeAttach struct{ Host AttachedHostRecord }

func (ChangeAttach) recordChange() {}

type ChangeRename struct {
	Device webapi.DeviceID
	Name   string
}

func (ChangeRename) recordChange() {}

type ChangeDetach struct{ Device webapi.DeviceID }

func (ChangeDetach) recordChange() {}

type ChangeOutcome string

const (
	ChangeApplied         ChangeOutcome = "applied"
	ChangeMissing         ChangeOutcome = "missing"
	ChangeArchivedOutcome ChangeOutcome = "archived"
	ChangeNotAttached     ChangeOutcome = "not_attached"
	ChangeNameTaken       ChangeOutcome = "name_taken"
)

func (c *Control) ChangeConversation(ctx context.Context, id webapi.ConversationID, change RecordChange) (ChangeOutcome, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return "", sqliteError(err)
	}
	defer tx.Rollback()
	var archived bool
	err = tx.QueryRowContext(ctx, "SELECT archived FROM conversations WHERE id=?", id.String()).Scan(&archived)
	if errors.Is(err, sql.ErrNoRows) {
		return ChangeMissing, nil
	}
	if err != nil {
		return "", sqliteError(err)
	}
	if _, ok := change.(ChangeArchived); archived && !ok {
		return ChangeArchivedOutcome, nil
	}
	switch v := change.(type) {
	case ChangeArchived:
		_, err = tx.ExecContext(ctx, "UPDATE conversations SET archived=? WHERE id=?", bool(v), id.String())
	case ChangeTitle:
		_, err = tx.ExecContext(ctx, "UPDATE conversations SET title=?1,title_origin='user' WHERE id=?2 AND title<>?1", string(v), id.String())
	case ChangePinned:
		_, err = tx.ExecContext(ctx, "UPDATE conversations SET pinned=? WHERE id=?", bool(v), id.String())
	case ChangeModel:
		var document []byte
		document, err = json.Marshal(v.Model)
		if err == nil {
			_, err = tx.ExecContext(ctx, "UPDATE conversations SET model=? WHERE id=?", string(document), id.String())
		}
	case ChangeAttach:
		var added bool
		added, err = insertAttachedHost(ctx, tx, id, v.Host, c.clock.Now())
		if err == nil && added {
			err = advanceContext(ctx, tx, id)
		}
	case ChangeDetach:
		var result sql.Result
		result, err = tx.ExecContext(ctx, "DELETE FROM conversation_hosts WHERE conversation_id=? AND device_id=?", id.String(), v.Device.String())
		if err == nil {
			var count int64
			count, err = result.RowsAffected()
			if err == nil && count > 0 {
				err = advanceContext(ctx, tx, id)
			}
		}
	case ChangeRename:
		var attached, taken bool
		err = tx.QueryRowContext(ctx, `SELECT EXISTS(SELECT 1 FROM conversation_hosts WHERE conversation_id=?1 AND device_id=?2),EXISTS(SELECT 1 FROM conversation_hosts WHERE conversation_id=?1 AND device_id<>?2 AND name=?3)`, id.String(), v.Device.String(), v.Name).Scan(&attached, &taken)
		if err != nil {
			break
		}
		if !attached {
			return ChangeNotAttached, nil
		}
		if taken {
			return ChangeNameTaken, nil
		}
		_, err = tx.ExecContext(ctx, "UPDATE conversation_hosts SET name=? WHERE conversation_id=? AND device_id=?", v.Name, id.String(), v.Device.String())
		if err == nil {
			err = advanceContext(ctx, tx, id)
		}
	default:
		return "", errors.New("unknown conversation change")
	}
	if err != nil {
		return "", sqliteError(err)
	}
	return ChangeApplied, sqliteError(tx.Commit())
}
