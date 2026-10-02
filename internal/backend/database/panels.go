package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/webapi"
)

// Panel returns the conversation's saved panel; nil when it never saved one. A
// stored document that no longer matches the panel's shape is corrupt,
// not repaired.
func (c *ControlService) Panel(ctx context.Context, conversation webapi.ConversationID) (*webapi.WorkPanel, error) {
	panic("not written: b-database")
}

// SavePanel replaces the conversation's panel with `document`, the panel's JSON.
func (c *ControlService) SavePanel(ctx context.Context, conversation webapi.ConversationID, document string) error {
	panic("not written: b-database")
}
