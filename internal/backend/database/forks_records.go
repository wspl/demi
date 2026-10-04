package database

import (
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// ForkOperation describes one creation attempt.
type ForkOperation struct {
	// The destination's id, which the attempt reserves.
	ID     webapiproto.ConversationID
	Owner  webapiproto.UserID
	Source webapiproto.ConversationID
	// The completed text the history is kept through.
	Block    types.BlockID
	Metadata ForkMetadata
}
