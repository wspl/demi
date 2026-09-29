package webapi

import (
	"github.com/wspl/demi/go/core"
)

// A workspace as the page lists it.
//
//demi:wire open
type WorkspaceDTO struct {
	ID       WorkspaceID `json:"id" check:"func=Validate"`
	DeviceID DeviceID    `json:"deviceId" check:"func=Validate"`
	// The directory on the device, absolute as the device names it.
	Path      string         `json:"path"`
	Name      string         `json:"name"`
	CreatedAt core.Timestamp `json:"createdAt" check:"func=core.Validate"`
}

// `GET /workspaces`: the user's workspaces, in their sidebar order.
//
//demi:wire open
type Workspaces struct {
	Workspaces []WorkspaceDTO `json:"workspaces"`
}

// `{ workspace }`: the answer of a creation and a rename.
//
//demi:wire open
type WorkspaceAnswer struct {
	Workspace WorkspaceDTO `json:"workspace"`
}

// `POST /workspaces`: a directory on one of the user's devices, or a new
// project directory on the user's Cloud, which can wake it.
//
//demi:union tag=kind
//demi:export
type CreateWorkspace interface{ isCreateWorkspace() }

//demi:variant device
type CreateWorkspaceDevice struct {
	DeviceID DeviceID `json:"deviceId" check:"func=Validate"`
	// An `AbsolutePath` is valid once it is decoded.
	Path AbsolutePath `json:"path" check:"func=Validate"`
	Name Trimmed      `json:"name" check:"chars=1..256,func=Validate"`
}

func (CreateWorkspaceDevice) isCreateWorkspace() {}

//demi:variant cloud
type CreateWorkspaceCloud struct {
	Name Trimmed `json:"name" check:"chars=1..256,func=Validate"`
}

func (CreateWorkspaceCloud) isCreateWorkspace() {}

// `PATCH /workspaces/:id`: a new name.
//
//demi:wire
type RenameWorkspace struct {
	Name Trimmed `json:"name" check:"chars=1..256,func=Validate"`
}
