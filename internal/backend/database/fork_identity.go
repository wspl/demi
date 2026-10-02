package database

import (
	"strings"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// SameAttempt reports whether owner, source and block identify this attempt.
func (o ForkOperation) SameAttempt(owner webapi.UserID, source webapi.ConversationID, block core.BlockID) bool {
	return o.Owner == owner && strings.EqualFold(string(o.Source), string(source)) && o.Block == block
}
