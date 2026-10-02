package webapi

import (
	"encoding/json"
)

// The product state: the signed-in user, the instance mode, the user's
// preferences, the provider entries the user infers with, the user's
// workspaces in their order, the user's devices, the paired ones and the
// Cloud, the backend's public URL, the summaries of the user's
// conversations, the active ones first, then the archived, the Cloud's
// status, the backend's plugins with whether the user has each on, and the
// state of each plugin the user has on that gives one, by its id. The backend
// reads it for each channel, without waking a Cloud or running inference.
// +demi:root direction=receive output=web
// +demi:tolerant
type ProductState struct {
	User        UserDTO         `json:"user"`
	Mode        InstanceMode    `json:"mode"`
	Preferences Preferences     `json:"preferences"`
	Providers   []ProviderState `json:"providers"`
	Workspaces  []WorkspaceDTO  `json:"workspaces"`
	Devices     []DeviceDTO     `json:"devices"`
	// The URL runners connect to (`DEMI_BACKEND_PUBLIC_URL`), whose origin
	// serves the installers: the page's install command names it, since
	// the page's own origin may be another server's, as in development.
	PublicURL     string                `json:"publicUrl"`
	Conversations []ConversationSummary `json:"conversations"`
	Cloud         CloudStatus           `json:"cloud"`
	// The backend's plugins, in their order of registration.
	Plugins []PluginEntry `json:"plugins"`
	// The state of each plugin the user has on that gives one, valid
	// against the schema its page package's types are generated from.
	PluginStates map[string]json.RawMessage `json:"pluginStates"`
}

// A message of the page's synchronization channel, `WS /sync`: the whole
// product state first, then the current value of each part of it that
// changed, each part's in the order of its changes.
// +demi:root direction=receive output=web
// +demi:union tag=type
//
//sumtype:decl
type SyncEvent interface{ syncEvent() }

// The whole product state, first on every connection; it replaces
// whatever the page held.
// +demi:variant SyncEvent snapshot
// +demi:tolerant
type SyncEventSnapshot struct {
	State ProductState `json:"state"`
}

// A conversation's summary, as the conversation lists carry it.
// +demi:variant SyncEvent conversation
// +demi:tolerant
type SyncEventConversation struct {
	Conversation ConversationSummary `json:"conversation"`
}

// The id of every conversation, in the product state's order.
// +demi:variant SyncEvent conversation_order
// +demi:tolerant
type SyncEventConversationOrder struct {
	IDs []ConversationID `json:"ids"`
}

// +demi:variant SyncEvent preferences
// +demi:tolerant
type SyncEventPreferences struct {
	Preferences Preferences `json:"preferences"`
}

// +demi:variant SyncEvent user
// +demi:tolerant
type SyncEventUser struct {
	User UserDTO `json:"user"`
}

// The user's workspaces, in the user's order.
// +demi:variant SyncEvent workspaces
// +demi:tolerant
type SyncEventWorkspaces struct {
	Workspaces []WorkspaceDTO `json:"workspaces"`
}

// The paired devices and the Cloud's.
// +demi:variant SyncEvent devices
// +demi:tolerant
type SyncEventDevices struct {
	Devices []DeviceDTO `json:"devices"`
}

// The user turned a plugin on or off.
// +demi:variant SyncEvent plugins
// +demi:tolerant
type SyncEventPlugins struct {
	Plugins []PluginEntry `json:"plugins"`
}

// A plugin's state for the user's pages changed.
// +demi:variant SyncEvent plugin
// +demi:tolerant
type SyncEventPlugin struct {
	Plugin string          `json:"plugin"`
	State  json.RawMessage `json:"state"`
}

// The entries the user infers with, each with what its provider says.
// +demi:variant SyncEvent providers
// +demi:tolerant
type SyncEventProviders struct {
	Providers []ProviderState `json:"providers"`
}

// +demi:variant SyncEvent cloud
// +demi:tolerant
type SyncEventCloud struct {
	Cloud CloudStatus `json:"cloud"`
}

// Nothing else was sent for 30 seconds.
// +demi:variant SyncEvent heartbeat
// +demi:tolerant
type SyncEventHeartbeat struct {
}
