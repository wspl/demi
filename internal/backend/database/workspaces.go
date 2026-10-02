package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/webapi"
)

// Workspace returns the workspace with id, or nil when absent.
func (c *ControlService) Workspace(ctx context.Context, id webapi.WorkspaceID) (*WorkspaceRecord, error) {
	panic("not written: b-database")
}

// Workspaces returns the user's workspaces, in their order.
func (c *ControlService) Workspaces(ctx context.Context, user webapi.UserID) ([]WorkspaceRecord, error) {
	panic("not written: b-database")
}

// CreateWorkspace returns the new workspace `id` at `path` on the user's `device`, after the
// user's others; nil, writing nothing, when the device is not the
// user's. The device is looked up in the same statement, so a
// revocation cannot slip between the check and the write.
func (c *ControlService) CreateWorkspace(ctx context.Context, id webapi.WorkspaceID, user webapi.UserID, device webapi.DeviceID, path string, name string) (*WorkspaceRecord, error) {
	panic("not written: b-database")
}

// RenameWorkspace renames the user's workspace `id` and answers it as it now is; nil
// when the user has no workspace of the id.
func (c *ControlService) RenameWorkspace(ctx context.Context, user webapi.UserID, id webapi.WorkspaceID, name string) (*WorkspaceRecord, error) {
	panic("not written: b-database")
}

// DeleteWorkspace deletes the user's workspace `id` unless conversations still target
// it; the count and the delete are one transaction.
func (c *ControlService) DeleteWorkspace(ctx context.Context, user webapi.UserID, id webapi.WorkspaceID) (WorkspaceDeletion, error) {
	panic("not written: b-database")
}
