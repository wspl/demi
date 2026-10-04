package runners

import (
	"context"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/webapiproto"
)

// DefaultLocale is the locale commands receive until the user's browser reports one.
func DefaultLocale() commandproto.CommandLocale {
	return commandproto.CommandLocale{TimeZone: "UTC", Languages: []commandproto.LanguageTag{"en-US"}}
}

// CommandContext builds the context of caller's work for conversation and its
// owner user, with the user's reported locale or the default before one.
func CommandContext(
	ctx context.Context,
	control *database.ControlService,
	user webapiproto.UserID,
	conversation webapiproto.ConversationID,
	caller commandproto.Caller,
) (commandproto.Context, error) {
	return workContext(ctx, control, user, string(conversation), caller)
}

// ProviderContext builds the context of user's work for a provider entry outside
// a conversation, such as installing its CLI; the context names the entry.
func ProviderContext(
	ctx context.Context,
	control *database.ControlService,
	user webapiproto.UserID,
	provider webapiproto.ProviderID,
) (commandproto.Context, error) {
	return workContext(ctx, control, user, "provider-"+string(provider), &commandproto.UserCaller{})
}

// workContext supplies the user's saved locale to a command's scope and caller.
func workContext(
	ctx context.Context,
	control *database.ControlService,
	user webapiproto.UserID,
	scope string,
	caller commandproto.Caller,
) (commandproto.Context, error) {
	preferences, err := control.Preferences(ctx, user)
	if err != nil {
		return commandproto.Context{}, err
	}
	locale := DefaultLocale()
	if preferences.Locale != nil {
		locale = *preferences.Locale
	}
	return commandproto.Context{Conversation: scope, Caller: caller, Locale: locale}, nil
}
