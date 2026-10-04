package webapiproto

// `POST /sidebar/reorder`: the row `id` moves before `beforeId`, or to the
// end of its partition when that is null. A conversation moves within its
// project and pin partition; a workspace among the user's workspaces.
// +demi:root direction=send output=web
// +demi:union tag=kind
//
//sumtype:decl
type SidebarReorder interface{ sidebarReorder() }

// +demi:variant SidebarReorder conversation
type SidebarReorderConversation struct {
	ID ConversationID `json:"id"`
	// +demi:nullable
	BeforeID *ConversationID `json:"beforeId"`
}

// +demi:variant SidebarReorder workspace
type SidebarReorderWorkspace struct {
	ID WorkspaceID `json:"id"`
	// +demi:nullable
	BeforeID *WorkspaceID `json:"beforeId"`
}
