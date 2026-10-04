package database

import (
	"strings"

	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// SameAttempt reports whether owner, source and block identify this attempt.
func (o ForkOperation) SameAttempt(
	owner webapiproto.UserID,
	source webapiproto.ConversationID,
	block types.BlockID,
) bool {
	return o.Owner == owner && strings.EqualFold(string(o.Source), string(source)) && o.Block == block
}
