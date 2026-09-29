package webapi

// `POST /sidebar/reorder`: the row `id` moves before `beforeId`, or to the
// end of its partition when that is null. A conversation moves within its
// project and pin partition; a workspace among the user's workspaces.
//
//demi:union tag=kind
//demi:export
type SidebarReorder interface{ isSidebarReorder() }

//demi:variant conversation
type SidebarReorderConversation struct {
	ID       ConversationID  `json:"id" check:"func=Validate"`
	BeforeID *ConversationID `json:"beforeId" check:"nullable,func=Validate"`
}

func (SidebarReorderConversation) isSidebarReorder() {}

//demi:variant workspace
type SidebarReorderWorkspace struct {
	ID       WorkspaceID  `json:"id" check:"func=Validate"`
	BeforeID *WorkspaceID `json:"beforeId" check:"nullable,func=Validate"`
}

func (SidebarReorderWorkspace) isSidebarReorder() {}
