package storage

import (
	"context"
	"database/sql"
	"errors"

	"github.com/google/uuid"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type WorkspaceRecord struct {
	ID         webapi.WorkspaceID
	User       webapi.UserID
	Device     webapi.DeviceID
	Path, Name string
	CreatedAt  core.Timestamp
}

func (r WorkspaceRecord) DTO() webapi.WorkspaceDTO {
	return webapi.WorkspaceDTO{ID: r.ID, DeviceID: r.Device, Path: r.Path, Name: r.Name, CreatedAt: r.CreatedAt}
}
func NewWorkspaceID() webapi.WorkspaceID {
	id, err := webapi.ParseWorkspaceID(uuid.NewString())
	if err != nil {
		panic(err)
	} // A generated UUID satisfies the workspace identity rule.
	return id
}

type WorkspaceDeletion struct {
	Kind      WorkspaceDeletionKind
	Targeting uint64
}
type WorkspaceDeletionKind string

const (
	WorkspaceDeleted WorkspaceDeletionKind = "deleted"
	WorkspaceMissing WorkspaceDeletionKind = "missing"
	WorkspaceInUse   WorkspaceDeletionKind = "in_use"
)
const workspaceColumns = "id,user_id,device_id,path,name,created_at"

func scanWorkspace(row scanner) (*WorkspaceRecord, error) {
	var r WorkspaceRecord
	var id, user, device string
	var ms int64
	err := row.Scan(&id, &user, &device, &r.Path, &r.Name, &ms)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	r.ID, err = webapi.ParseWorkspaceID(id)
	if err != nil {
		return nil, corrupt("workspaces", "id", err)
	}
	r.User, err = webapi.ParseUserID(user)
	if err != nil {
		return nil, corrupt("workspaces", "user_id", err)
	}
	r.Device, err = webapi.ParseDeviceID(device)
	if err != nil {
		return nil, corrupt("workspaces", "device_id", err)
	}
	r.CreatedAt, err = instant("workspaces", "created_at", ms)
	return &r, err
}
func (c *Control) Workspace(ctx context.Context, id webapi.WorkspaceID) (*WorkspaceRecord, error) {
	return scanWorkspace(c.db.QueryRowContext(ctx, "SELECT "+workspaceColumns+" FROM workspaces WHERE id=?", id.String()))
}
func (c *Control) Workspaces(ctx context.Context, user webapi.UserID) ([]WorkspaceRecord, error) {
	rows, err := c.db.QueryContext(ctx, "SELECT "+workspaceColumns+" FROM workspaces WHERE user_id=? ORDER BY sort_order,id", user.String())
	if err != nil {
		return nil, sqliteError(err)
	}
	defer rows.Close()
	records := []WorkspaceRecord{}
	for rows.Next() {
		r, err := scanWorkspace(rows)
		if err != nil {
			return nil, err
		}
		records = append(records, *r)
	}
	return records, sqliteError(rows.Err())
}
func (c *Control) CreateWorkspace(ctx context.Context, id webapi.WorkspaceID, user webapi.UserID, device webapi.DeviceID, path, name string) (*WorkspaceRecord, error) {
	return scanWorkspace(c.db.QueryRowContext(ctx, `INSERT INTO workspaces(id,user_id,device_id,path,name,sort_order,created_at)
 SELECT ?1,?2,?3,?4,?5,(SELECT COALESCE(MAX(sort_order),-1)+1 FROM workspaces WHERE user_id=?2),?6
 WHERE EXISTS(SELECT 1 FROM devices WHERE id=?3 AND user_id=?2) RETURNING `+workspaceColumns, id.String(), user.String(), device.String(), path, name, c.clock.Now().Millisecond()))
}
func (c *Control) RenameWorkspace(ctx context.Context, user webapi.UserID, id webapi.WorkspaceID, name string) (*WorkspaceRecord, error) {
	return scanWorkspace(c.db.QueryRowContext(ctx, "UPDATE workspaces SET name=? WHERE id=? AND user_id=? RETURNING "+workspaceColumns, name, id.String(), user.String()))
}
func (c *Control) DeleteWorkspace(ctx context.Context, user webapi.UserID, id webapi.WorkspaceID) (WorkspaceDeletion, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return WorkspaceDeletion{}, sqliteError(err)
	}
	defer tx.Rollback()
	var found bool
	if err = tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM workspaces WHERE id=? AND user_id=?)", id.String(), user.String()).Scan(&found); err != nil {
		return WorkspaceDeletion{}, sqliteError(err)
	}
	if !found {
		return WorkspaceDeletion{Kind: WorkspaceMissing}, nil
	}
	var targeting uint64
	if err = tx.QueryRowContext(ctx, "SELECT COUNT(*) FROM conversations WHERE target_workspace_id=?", id.String()).Scan(&targeting); err != nil {
		return WorkspaceDeletion{}, sqliteError(err)
	}
	if targeting > 0 {
		return WorkspaceDeletion{WorkspaceInUse, targeting}, nil
	}
	if _, err = tx.ExecContext(ctx, "DELETE FROM workspaces WHERE id=?", id.String()); err != nil {
		return WorkspaceDeletion{}, sqliteError(err)
	}
	return WorkspaceDeletion{Kind: WorkspaceDeleted}, sqliteError(tx.Commit())
}
