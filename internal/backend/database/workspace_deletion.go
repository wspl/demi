package database

import (
	"github.com/google/uuid"
	"github.com/wspl/demi/internal/webapi"
)

// WorkspaceDeletion describes what deleting a workspace found.
//
//sumtype:decl
type WorkspaceDeletion interface{ workspaceDeletion() }

// WorkspaceDeleted means the workspace was deleted.
type WorkspaceDeleted struct{}

func (*WorkspaceDeleted) workspaceDeletion() {}

// WorkspaceMissing means the user has no workspace of this ID.
type WorkspaceMissing struct{}

func (*WorkspaceMissing) workspaceDeletion() {}

// WorkspaceInUse counts conversations still targeting the workspace.
type WorkspaceInUse struct{ Count uint64 }

func (*WorkspaceInUse) workspaceDeletion() {}

// NewWorkspaceID returns a new workspace ID assigned by the backend.
func NewWorkspaceID() webapi.WorkspaceID {
	return webapi.WorkspaceID(uuid.NewString())
}

// DTO returns the workspace as the page lists it.
func (w WorkspaceRecord) DTO() webapi.WorkspaceDTO {
	return webapi.WorkspaceDTO{ID: w.ID, DeviceID: w.Device, Path: w.Path, Name: w.Name, CreatedAt: w.CreatedAt}
}
