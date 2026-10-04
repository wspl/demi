package database

import (
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

//revive:disable:exported
// Contract doc comments describe the stored JSON, in the form contractgen uses
// for schema descriptions.

// A device attached to a conversation (`sessions-and-targets.md`
// § Attached hosts): a Host the conversation reaches besides its main one.
// A Fork keeps its source's in its operation's metadata.
// +demi:root
type AttachedHostRecord struct {
	Device webapiproto.DeviceID `json:"device"`
	// What the model and the user call the host; unique within the
	// conversation.
	// +demi:length chars min=1
	Name string `json:"name"`
	// Where the last `demi host shell --host` there ended; none until one
	// ran.
	// +demi:nullable
	CWD *string `json:"cwd"`
}

// The latest target switch, which every node's next context block
// describes.
// +demi:root
// +demi:tolerant
type TargetSwitch struct {
	From ExecutionTarget `json:"from"`
	To   ExecutionTarget `json:"to"`
}

// What the destination is published with.
// +demi:root
type ForkMetadata struct {
	// +demi:length chars min=1
	Title  string                         `json:"title"`
	Target webapiproto.ConversationTarget `json:"target"`
	// The model selection the source's record held, which the destination
	// inherits; none when the source had none.
	// +demi:nullable
	Model         *types.ModelSelection `json:"model"`
	CreatedAt     types.Timestamp       `json:"createdAt"`
	AttachedHosts []AttachedHostRecord  `json:"attachedHosts"`
}

// An entry's cached catalog as the control store keeps it: model metadata
// only, under the key of the configuration and account it was read for.
// +demi:root
type CatalogRecord struct {
	// The digest of the entry's configuration and active account.
	// +demi:length chars min=1
	Key string `json:"key"`
	// When the source last answered.
	CheckedAt types.Timestamp         `json:"checkedAt"`
	Catalog   types.ProviderModelList `json:"catalog"`
}

// A conversation's selection resolved: the device its work runs on, and
// the directory the work starts in there. A Cloud has no device until its
// first use makes it.
// +demi:root
// +demi:union tag=kind
//
//sumtype:decl
type ExecutionTarget interface{ executionTarget() }

// +demi:variant ExecutionTarget cloud
// +demi:tolerant
type ExecutionCloud struct {
	// +demi:nullable
	DeviceID *webapiproto.DeviceID `json:"deviceId"`
	Path     string                `json:"path"`
}

// +demi:variant ExecutionTarget device
// +demi:tolerant
type ExecutionDevice struct {
	DeviceID webapiproto.DeviceID `json:"deviceId"`
	Path     string               `json:"path"`
}

// +demi:variant ExecutionTarget workspace
// +demi:tolerant
type ExecutionWorkspace struct {
	WorkspaceID webapiproto.WorkspaceID `json:"workspaceId"`
	DeviceID    webapiproto.DeviceID    `json:"deviceId"`
	Path        string                  `json:"path"`
}
