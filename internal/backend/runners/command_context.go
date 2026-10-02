package runners

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/webapi"
)

// DefaultLocale is the locale commands receive until the user's browser reports one.
func DefaultLocale() commandwire.CommandLocale {
	return commandwire.CommandLocale{TimeZone: "UTC", Languages: []commandwire.LanguageTag{"en-US"}}
}

// CommandContext builds the context of caller's work for conversation and its
// owner user, with the user's reported locale or the default before one.
func CommandContext(ctx context.Context, control *database.ControlService, user webapi.UserID, conversation webapi.ConversationID, caller commandwire.CommandCaller) (commandwire.CommandContext, error) {
	return workContext(ctx, control, user, string(conversation), caller)
}

// ProviderContext builds the context of user's work for a provider entry outside
// a conversation, such as installing its CLI; the context names the entry.
func ProviderContext(ctx context.Context, control *database.ControlService, user webapi.UserID, provider webapi.ProviderID) (commandwire.CommandContext, error) {
	return workContext(ctx, control, user, "provider-"+string(provider), &commandwire.UserCaller{})
}

// workContext supplies the user's saved locale to a command's scope and caller.
func workContext(ctx context.Context, control *database.ControlService, user webapi.UserID, scope string, caller commandwire.CommandCaller) (commandwire.CommandContext, error) {
	preferences, err := control.Preferences(ctx, user)
	if err != nil {
		return commandwire.CommandContext{}, err
	}
	locale := DefaultLocale()
	if preferences.Locale != nil {
		locale = *preferences.Locale
	}
	return commandwire.CommandContext{Conversation: scope, Caller: caller, Locale: locale}, nil
}
