package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/webapi"
)

// Draft returns the conversation's draft: the empty one at revision 0 before its first
// save.
func (c *ControlService) Draft(ctx context.Context, conversation webapi.ConversationID) (webapi.ConversationDraft, error) {
	panic("not written: b-database")
}

// SaveDraft saves `text` and `files` as the draft of `owner`'s conversation. The
// save always takes effect; when `base` is not the current revision, the
// version it replaces is kept as the replaced one, unless that version
// is empty or the one saved. Each upload is presented with what its
// record holds.
func (c *ControlService) SaveDraft(ctx context.Context, conversation webapi.ConversationID, owner webapi.UserID, base uint64, text string, files []StagedFile) (webapi.ConversationDraft, error) {
	panic("not written: b-database")
}

// ChangeReplacedDraft restores or dismisses the replaced version of `revision`. A restore
// exchanges it with the draft, whose version becomes the replaced one
// unless it is empty.
func (c *ControlService) ChangeReplacedDraft(ctx context.Context, conversation webapi.ConversationID, action webapi.ReplacedAction, revision uint64) (webapi.ConversationDraft, error) {
	panic("not written: b-database")
}
