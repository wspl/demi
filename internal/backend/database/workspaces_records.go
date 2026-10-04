package database

import (
	"github.com/google/uuid"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// WorkspaceRecord is a `workspaces` row.
type WorkspaceRecord struct {
	ID        webapiproto.WorkspaceID
	User      webapiproto.UserID
	Device    webapiproto.DeviceID
	Path      string
	Name      string
	CreatedAt types.Timestamp
}

// NewWorkspaceID returns a new workspace ID assigned by the backend.
func NewWorkspaceID() webapiproto.WorkspaceID {
	return webapiproto.WorkspaceID(uuid.NewString())
}

// DTO returns the workspace as the page lists it.
func (w WorkspaceRecord) DTO() webapiproto.WorkspaceDTO {
	return webapiproto.WorkspaceDTO{ID: w.ID, DeviceID: w.Device, Path: w.Path, Name: w.Name, CreatedAt: w.CreatedAt}
}
