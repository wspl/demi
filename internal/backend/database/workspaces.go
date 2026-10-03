package database

import (
	"context"
	"database/sql"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// Workspace returns the workspace with id, or nil when absent.
func (c *ControlService) Workspace(ctx context.Context, id webapi.WorkspaceID) (WorkspaceRecord, bool, error) {
	var found bool
	record, err := controlCall(
		ctx,
		c,
		func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (WorkspaceRecord, error) {
			r, ok, err := queryRecord(ctx, tx, "workspaces", "SELECT * FROM workspaces WHERE id = ?", workspaceRow, id)
			found = ok
			return r, err
		},
	)
	return record, found && err == nil, err
}

// Workspaces returns the user's workspaces, in their order.
func (c *ControlService) Workspaces(ctx context.Context, user webapi.UserID) ([]WorkspaceRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) ([]WorkspaceRecord, error) {
		return queryRecords(
			ctx,
			tx,
			"workspaces",
			"SELECT * FROM workspaces WHERE user_id = ? ORDER BY sort_order,id",
			workspaceRow,
			user,
		)
	})
}

// CreateWorkspace returns the new workspace `id` at `path` on the user's `device`, after the
// user's others; nil, writing nothing, when the device is not the
// user's. The device is looked up in the same statement, so a
// revocation cannot slip between the check and the write.
func (c *ControlService) CreateWorkspace(
	ctx context.Context,
	id webapi.WorkspaceID,
	user webapi.UserID,
	device webapi.DeviceID,
	path string,
	name string,
) (*WorkspaceRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now core.Timestamp) (*WorkspaceRecord, error) {
		at, err := now.Millisecond()
		if err != nil {
			return nil, err
		}
		record, found, err := queryRecord(
			ctx,
			tx,
			"workspaces",
			`INSERT INTO workspaces (id,user_id,device_id,path,name,sort_order,created_at)
SELECT ?1,?2,?3,?4,?5,(SELECT COALESCE(MAX(sort_order),-1)+1 FROM workspaces WHERE user_id=?2),?6
WHERE EXISTS (SELECT 1 FROM devices WHERE id=?3 AND user_id=?2)
RETURNING *`,
			workspaceRow,
			id,
			user,
			device,
			path,
			name,
			at,
		)
		if err != nil || !found {
			return nil, err
		}
		return &record, nil
	})
}

// RenameWorkspace renames the user's workspace `id` and answers it as it now is; nil
// when the user has no workspace of the id.
func (c *ControlService) RenameWorkspace(
	ctx context.Context,
	user webapi.UserID,
	id webapi.WorkspaceID,
	name string,
) (*WorkspaceRecord, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (*WorkspaceRecord, error) {
		record, found, err := queryRecord(
			ctx,
			tx,
			"workspaces",
			"UPDATE workspaces SET name = ? WHERE id = ? AND user_id = ? RETURNING *",
			workspaceRow,
			name,
			id,
			user,
		)
		if err != nil || !found {
			return nil, err
		}
		return &record, nil
	})
}

// DeleteWorkspace deletes the user's workspace `id` unless conversations still target
// it; the count and the delete are one transaction.
func (c *ControlService) DeleteWorkspace(
	ctx context.Context,
	user webapi.UserID,
	id webapi.WorkspaceID,
) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) error {
		var found bool
		if err := tx.QueryRowContext(ctx, "SELECT EXISTS (SELECT 1 FROM workspaces WHERE id = ? AND user_id = ?)", id, user).
			Scan(&found); err != nil {
			return err
		}
		if !found {
			return ErrWorkspaceNotFound
		}
		var count uint64
		if err := tx.QueryRowContext(ctx, "SELECT COUNT(*) FROM conversations WHERE target_workspace_id = ?", id).
			Scan(&count); err != nil {
			return err
		}
		if count > 0 {
			return &WorkspaceInUseError{Count: count}
		}
		return execSQL(ctx, tx, "DELETE FROM workspaces WHERE id = ?", id)
	})
}

func workspaceRow(r *storedRow) WorkspaceRecord {
	return WorkspaceRecord{
		ID:        checked(r, "id", webapi.ParseWorkspaceID),
		User:      checked(r, "user_id", webapi.ParseUserID),
		Device:    checked(r, "device_id", webapi.ParseDeviceID),
		Path:      r.text("path"),
		Name:      r.text("name"),
		CreatedAt: r.instant("created_at"),
	}
}
