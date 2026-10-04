package database

import (
	"context"
	"database/sql"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// Panel returns the conversation's panel, empty before its first change.
func (c *ControlService) Panel(ctx context.Context, conversation webapi.ConversationID) (webapi.WorkPanel, error) {
	return controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, _ core.Timestamp) (webapi.WorkPanel, error) {
		panel, err := readPanel(ctx, tx, conversation)
		return webapi.WorkPanel{Revision: panel.revision, Tabs: panel.document.Tabs}, err
	})
}

type storedPanel struct {
	revision uint64
	document panelDocument
}

func readPanel(ctx context.Context, tx *sql.Tx, conversation webapi.ConversationID) (storedPanel, error) {
	panel, found, err := queryRecord(ctx, tx, "conversation_panels",
		"SELECT revision, document FROM conversation_panels WHERE conversation_id = ?",
		func(r *storedRow) storedPanel {
			return storedPanel{revision: r.count("revision"), document: storedJSON(r, "document", decodePanelDocument)}
		}, conversation)
	if !found && err == nil {
		panel.document = panelDocument{Tabs: []webapi.PanelTab{}, Retired: []string{}}
	}
	return panel, err
}

// ChangePanel applies one change in a transaction, returning the revision and
// effect. Changed is false when the panel already satisfied the operation.
func (c *ControlService) ChangePanel(
	ctx context.Context,
	conversation webapi.ConversationID,
	change PanelChange,
) (uint64, PanelEffect, bool, error) {
	return c.changePanel(ctx, conversation, change, panelScope{})
}

// ChangePanelOfKinds atomically restricts a change to the supplied kinds. An
// absent tab remains a no-op, while a concurrent creation is checked in the
// same transaction that applies the change.
func (c *ControlService) ChangePanelOfKinds(
	ctx context.Context,
	conversation webapi.ConversationID,
	change PanelChange,
	kinds []string,
) (uint64, PanelEffect, bool, error) {
	return c.changePanel(ctx, conversation, change, panelScope{restricted: true, kinds: kinds})
}

func (c *ControlService) changePanel(
	ctx context.Context,
	conversation webapi.ConversationID,
	change PanelChange,
	scope panelScope,
) (uint64, PanelEffect, bool, error) {
	type outcome struct {
		revision uint64
		effect   PanelEffect
		changed  bool
	}
	result, err := controlCall(ctx, c, func(ctx context.Context, tx *sql.Tx, now core.Timestamp) (outcome, error) {
		if err := draftWritable(ctx, tx, conversation); err != nil {
			return outcome{}, err
		}
		panel, err := readPanel(ctx, tx, conversation)
		if err != nil {
			return outcome{}, err
		}
		if err := panel.document.checkKinds(change, scope); err != nil {
			return outcome{}, err
		}
		effect, changed, err := panel.document.apply(change)
		if err != nil {
			return outcome{}, err
		}
		if !changed {
			return outcome{revision: panel.revision}, nil
		}
		panel.revision++
		document, err := encoded(panel.document)
		if err != nil {
			return outcome{}, err
		}
		at, err := now.Millisecond()
		if err != nil {
			return outcome{}, err
		}
		err = execSQL(ctx, tx, `INSERT INTO conversation_panels (conversation_id,revision,document,updated_at)
VALUES (?,?,?,?) ON CONFLICT (conversation_id) DO UPDATE
SET revision=excluded.revision,document=excluded.document,updated_at=excluded.updated_at`,
			conversation, panel.revision, document, at)
		return outcome{revision: panel.revision, effect: effect, changed: true}, err
	})
	return result.revision, result.effect, result.changed, err
}
