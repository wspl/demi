package webapi

// The product state: the signed-in user, the instance mode, the user's
// preferences, the provider entries the user infers with, the user's
// workspaces in their order, the user's devices, the paired ones and the
// Cloud, the user's live exposes, soonest expiry first, with the domain of
// their hostnames, null when the instance has none and exposes are off,
// the backend's public URL, the summaries of the user's conversations, the
// active ones first, then the archived, and the Cloud's status. The backend
// reads it for each channel, without waking a Cloud or running inference.
//
//demi:wire open
type ProductState struct {
	User         UserDTO         `json:"user"`
	Mode         InstanceMode    `json:"mode"`
	Preferences  Preferences     `json:"preferences"`
	Providers    []ProviderState `json:"providers"`
	Workspaces   []WorkspaceDTO  `json:"workspaces"`
	Devices      []DeviceDTO     `json:"devices"`
	Exposes      []ExposeDTO     `json:"exposes"`
	ExposeDomain *string         `json:"exposeDomain" check:"nullable"`
	// The URL runners connect to (`DEMI_BACKEND_PUBLIC_URL`), whose origin
	// serves the installers: the page's install command names it, since
	// the page's own origin may be another server's, as in development.
	PublicURL     string                `json:"publicUrl"`
	Conversations []ConversationSummary `json:"conversations"`
	Cloud         CloudStatus           `json:"cloud"`
}

// A message of the page's synchronization channel, `WS /sync`: the whole
// product state first, then the current value of each part of it that
// changed, each part's in the order of its changes.
//
//demi:union tag=type
//demi:export
type SyncEvent interface{ isSyncEvent() }

// The whole product state, first on every connection; it replaces
// whatever the page held.
//
//demi:variant snapshot open
type SyncEventSnapshot struct {
	State ProductState `json:"state"`
}

func (SyncEventSnapshot) isSyncEvent() {}

// A conversation's summary, as the conversation lists carry it.
//
//demi:variant conversation open
type SyncEventConversation struct {
	Conversation ConversationSummary `json:"conversation"`
}

func (SyncEventConversation) isSyncEvent() {}

// The id of every conversation, in the product state's order.
//
//demi:variant conversation_order open
type SyncEventConversationOrder struct {
	Ids []ConversationID `json:"ids" check:"each(func=Validate)"`
}

func (SyncEventConversationOrder) isSyncEvent() {}

//demi:variant preferences open
type SyncEventPreferences struct {
	Preferences Preferences `json:"preferences"`
}

func (SyncEventPreferences) isSyncEvent() {}

//demi:variant user open
type SyncEventUser struct {
	User UserDTO `json:"user"`
}

func (SyncEventUser) isSyncEvent() {}

// The user's workspaces, in the user's order.
//
//demi:variant workspaces open
type SyncEventWorkspaces struct {
	Workspaces []WorkspaceDTO `json:"workspaces"`
}

func (SyncEventWorkspaces) isSyncEvent() {}

// The paired devices and the Cloud's.
//
//demi:variant devices open
type SyncEventDevices struct {
	Devices []DeviceDTO `json:"devices"`
}

func (SyncEventDevices) isSyncEvent() {}

// The live exposes, soonest expiry first.
//
//demi:variant exposes open
type SyncEventExposes struct {
	Exposes []ExposeDTO `json:"exposes"`
}

func (SyncEventExposes) isSyncEvent() {}

// The entries the user infers with, each with what its provider says.
//
//demi:variant providers open
type SyncEventProviders struct {
	Providers []ProviderState `json:"providers"`
}

func (SyncEventProviders) isSyncEvent() {}

//demi:variant cloud open
type SyncEventCloud struct {
	Cloud CloudStatus `json:"cloud"`
}

func (SyncEventCloud) isSyncEvent() {}

// Nothing else was sent for 30 seconds.
//
//demi:variant heartbeat open
type SyncEventHeartbeat struct {
}

func (SyncEventHeartbeat) isSyncEvent() {}
