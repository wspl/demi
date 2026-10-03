package database

import (
	"github.com/google/uuid"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// WorkspaceRecord is a `workspaces` row.
type WorkspaceRecord struct {
	ID        webapi.WorkspaceID
	User      webapi.UserID
	Device    webapi.DeviceID
	Path      string
	Name      string
	CreatedAt core.Timestamp
}

// NewWorkspaceID returns a new workspace ID assigned by the backend.
func NewWorkspaceID() webapi.WorkspaceID {
	return webapi.WorkspaceID(uuid.NewString())
}

// DTO returns the workspace as the page lists it.
func (w WorkspaceRecord) DTO() webapi.WorkspaceDTO {
	return webapi.WorkspaceDTO{ID: w.ID, DeviceID: w.Device, Path: w.Path, Name: w.Name, CreatedAt: w.CreatedAt}
}
