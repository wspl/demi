package database

import (
	"context"
	"database/sql"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// Panel returns the conversation's saved panel; nil when it never saved one. A
// stored document that no longer matches the panel's shape is corrupt,
// not repaired.
func (c *ControlService) Panel(ctx context.Context, conversation webapi.ConversationID) (*webapi.WorkPanel, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (*webapi.WorkPanel, error) {
		return queryRecord(ctx, tx, "conversation_panels", "SELECT document FROM conversation_panels WHERE conversation_id = ?", func(r *storedRow) webapi.WorkPanel { return storedJSON(r, "document", webapi.DecodeWorkPanel) }, conversation)
	})
}

// SavePanel replaces the conversation's panel with `document`, the panel's JSON.
func (c *ControlService) SavePanel(ctx context.Context, conversation webapi.ConversationID, document string) error {
	return controlDo(ctx, c, func(ctx context.Context, tx *sql.Tx, now core.Timestamp) error {
		at, err := now.Millisecond()
		if err != nil {
			return err
		}
		return execSQL(ctx, tx, "INSERT INTO conversation_panels (conversation_id,document,updated_at) VALUES (?,?,?) ON CONFLICT (conversation_id) DO UPDATE SET document=excluded.document,updated_at=excluded.updated_at", conversation, document, at)
	})
}
