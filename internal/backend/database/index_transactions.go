package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"
	"database/sql"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// InsertConversation inserts new first in its owner's sidebar, unarchived and
// unpinned, unless its ID exists in any spelling. It returns the inserted count.
func InsertConversation(ctx context.Context, tx *sql.Tx, record NewConversation) (int, error) {
	panic("not written: b-database")
}

// ConversationByID reads a conversation in any spelling of its ID.
func ConversationByID(ctx context.Context, tx *sql.Tx, id webapi.ConversationID) (*ConversationRecord, error) {
	panic("not written: b-database")
}

// InsertAttachedHost attaches a new device under the first free name: name,
// name-2, name-3, and so on; an empty name becomes its ID. An existing device
// keeps its row and returns false. The caller owns the transaction.
func InsertAttachedHost(ctx context.Context, tx *sql.Tx, conversation webapi.ConversationID, host AttachedHostRecord, now core.Timestamp) (bool, error) {
	panic("not written: b-database")
}
