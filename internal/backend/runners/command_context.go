package runners

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/webapi"
)

// DefaultLocale is the locale commands receive until the user's browser reports one.
func DefaultLocale() commandwire.CommandLocale { panic("not written: b-runners") }

// CommandContext builds the context of caller's work for conversation and its
// owner user, with the user's reported locale or the default before one.
func CommandContext(ctx context.Context, control *database.ControlService, user webapi.UserID, conversation webapi.ConversationID, caller commandwire.CommandCaller) (commandwire.CommandContext, error) {
	panic("not written: b-runners")
}

// ProviderContext builds the context of user's work for a provider entry outside
// a conversation, such as installing its CLI; the context names the entry.
func ProviderContext(ctx context.Context, control *database.ControlService, user webapi.UserID, provider webapi.ProviderID) (commandwire.CommandContext, error) {
	panic("not written: b-runners")
}
