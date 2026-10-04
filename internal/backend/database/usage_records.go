package database

import (
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// UsageRow describes one answered request: who made it, in which conversation, with which
// entry and model, and the tokens its response reported.
type UsageRow struct {
	User         webapiproto.UserID
	Conversation webapiproto.ConversationID
	Provider     webapiproto.ProviderID
	Model        string
	Usage        types.TokenUsage
}
