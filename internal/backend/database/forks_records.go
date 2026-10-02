package database

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// ForkOperation describes one creation attempt.
type ForkOperation struct {
	// The destination's id, which the attempt reserves.
	ID     webapi.ConversationID
	Owner  webapi.UserID
	Source webapi.ConversationID
	// The completed text the history is kept through.
	Block    core.BlockID
	Metadata ForkMetadata
}
