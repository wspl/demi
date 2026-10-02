package database

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// UsageRow describes one answered request: who made it, in which conversation, with which
// entry and model, and the tokens its response reported.
type UsageRow struct {
	User         webapi.UserID
	Conversation webapi.ConversationID
	Provider     webapi.ProviderID
	Model        string
	Usage        core.TokenUsage
}
