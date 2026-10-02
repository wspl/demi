package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// SameAttempt reports whether owner, source and block identify this attempt.
func (o ForkOperation) SameAttempt(owner webapi.UserID, source webapi.ConversationID, block core.BlockID) bool {
	panic("not written: b-database")
}
