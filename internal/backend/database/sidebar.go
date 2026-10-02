package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/webapi"
)

// ReorderConversations moves the user's conversation `moved` before `before`, or to the end,
// within its partition (`storage.md` § Control records): the
// conversations that are not archived, pinned as it is, and in the same
// workspace or in none. False, writing nothing, when either is not in
// that partition, an archived one included.
func (c *ControlService) ReorderConversations(ctx context.Context, user webapi.UserID, moved webapi.ConversationID, before *webapi.ConversationID) (bool, error) {
	panic("not written: b-database")
}

// ReorderWorkspaces moves the user's workspace `moved` before `before`, or to the end;
// false when either is not one of the user's workspaces.
func (c *ControlService) ReorderWorkspaces(ctx context.Context, user webapi.UserID, moved webapi.WorkspaceID, before *webapi.WorkspaceID) (bool, error) {
	panic("not written: b-database")
}
