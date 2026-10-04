package webapiproto

import (
	"github.com/wspl/demi/internal/types"
)

// The most characters of a workspace's name.
const WorkspaceNameMax = 256

// A workspace as the page lists it.
// +demi:tolerant
type WorkspaceDTO struct {
	ID       WorkspaceID `json:"id"`
	DeviceID DeviceID    `json:"deviceId"`
	// The directory on the device, absolute as the device names it.
	Path      string          `json:"path"`
	Name      string          `json:"name"`
	CreatedAt types.Timestamp `json:"createdAt"`
}

// `GET /workspaces`: the user's workspaces, in their sidebar order.
// +demi:root direction=receive output=web
// +demi:tolerant
type Workspaces struct {
	Workspaces []WorkspaceDTO `json:"workspaces"`
}

// `{ workspace }`: the answer of a creation and a rename.
// +demi:root direction=receive output=web
// +demi:tolerant
type WorkspaceAnswer struct {
	Workspace WorkspaceDTO `json:"workspace"`
}

// `POST /workspaces`: a directory on one of the user's devices, or a new
// project directory on the user's Cloud, which can wake it.
// +demi:root direction=send output=web
// +demi:union tag=kind
//
//sumtype:decl
type CreateWorkspace interface{ createWorkspace() }

// +demi:variant CreateWorkspace device
type CreateWorkspaceDevice struct {
	DeviceID DeviceID `json:"deviceId"`
	// An `AbsolutePath` is valid once it is decoded.
	Path AbsolutePath `json:"path"`
	// +demi:length chars min=1 max=256
	Name Trimmed `json:"name"`
}

// +demi:variant CreateWorkspace cloud
type CreateWorkspaceCloud struct {
	// +demi:length chars min=1 max=256
	Name Trimmed `json:"name"`
}

// `PATCH /workspaces/:id`: a new name.
// +demi:root direction=send output=web
type RenameWorkspace struct {
	// +demi:length chars min=1 max=256
	Name Trimmed `json:"name"`
}
