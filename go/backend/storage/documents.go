package storage

import (
	"context"
	"database/sql"
	"encoding/json/v2"
	"errors"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/webapi"
)

// CheckedPatch carries preferences after the backend's locale check.
// CheckPreferences requires the caller's locale canonicalizer (ICU's IANA names
// and BCP 47 aliases in Rust); storage never silently repairs a stored document.
type CheckedPatch struct{ patch webapi.PreferencesPatch }

func CheckPreferences(patch webapi.PreferencesPatch, canonicalize func(commandservice.CommandLocale) (commandservice.CommandLocale, error)) (CheckedPatch, error) {
	if err := webapi.Validate(patch); err != nil {
		return CheckedPatch{}, err
	}
	if patch.Locale != nil {
		if canonicalize == nil {
			return CheckedPatch{}, errors.New("locale canonicalizer is required")
		}
		checked, err := canonicalize(*patch.Locale)
		if err != nil {
			return CheckedPatch{}, err
		}
		if err := commandservice.Validate(checked); err != nil {
			return CheckedPatch{}, err
		}
		patch.Locale = &checked
	}
	return CheckedPatch{patch}, nil
}
func preferences(ctx context.Context, db database, user webapi.UserID) (webapi.Preferences, error) {
	var text string
	err := db.QueryRowContext(ctx, "SELECT preferences FROM user_preferences WHERE user_id=?", user.String()).Scan(&text)
	if errors.Is(err, sql.ErrNoRows) {
		return webapi.Preferences{}, nil
	}
	if err != nil {
		return webapi.Preferences{}, sqliteError(err)
	}
	value, err := webapi.Decode[webapi.Preferences]([]byte(text))
	return value, corrupt("user_preferences", "preferences", err)
}
func (c *Control) Preferences(ctx context.Context, user webapi.UserID) (webapi.Preferences, error) {
	return preferences(ctx, c.db, user)
}
func (c *Control) PatchPreferences(ctx context.Context, user webapi.UserID, checked CheckedPatch) (webapi.Preferences, error) {
	tx, err := c.db.BeginTx(ctx, nil)
	if err != nil {
		return webapi.Preferences{}, sqliteError(err)
	}
	defer tx.Rollback()
	saved, err := preferences(ctx, tx, user)
	if err != nil {
		return saved, err
	}
	patch := checked.patch
	if p := patch.Appearance; p != nil {
		if p.Theme != nil {
			saved.Appearance.Theme = p.Theme
		}
		if p.Tone != nil {
			saved.Appearance.Tone = p.Tone
		}
		if p.Accent != nil {
			saved.Appearance.Accent = p.Accent
		}
		if p.FontSize != nil {
			saved.Appearance.FontSize = p.FontSize
		}
	}
	if p := patch.Shortcuts; p != nil {
		if p.New != nil {
			saved.Shortcuts.New = *p.New
		}
		if p.Sidebar != nil {
			saved.Shortcuts.Sidebar = *p.Sidebar
		}
		if p.Settings != nil {
			saved.Shortcuts.Settings = *p.Settings
		}
	}
	if patch.LastModel != nil {
		saved.LastModel = patch.LastModel
	}
	if patch.Locale != nil {
		saved.Locale = patch.Locale
	}
	document, err := json.Marshal(saved)
	if err != nil {
		return saved, err
	}
	_, err = tx.ExecContext(ctx, "INSERT INTO user_preferences(user_id,preferences) VALUES (?,?) ON CONFLICT(user_id) DO UPDATE SET preferences=excluded.preferences", user.String(), string(document))
	if err != nil {
		return saved, sqliteError(err)
	}
	return saved, sqliteError(tx.Commit())
}
func (c *Control) Panel(ctx context.Context, id webapi.ConversationID) (*webapi.WorkPanel, error) {
	var text string
	err := c.db.QueryRowContext(ctx, "SELECT document FROM conversation_panels WHERE conversation_id=?", id.String()).Scan(&text)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, sqliteError(err)
	}
	panel, err := webapi.Decode[webapi.WorkPanel]([]byte(text))
	if err != nil {
		return nil, corrupt("conversation_panels", "document", err)
	}
	return &panel, nil
}
func (c *Control) SavePanel(ctx context.Context, id webapi.ConversationID, document string) error {
	_, err := c.db.ExecContext(ctx, "INSERT INTO conversation_panels(conversation_id,document,updated_at) VALUES (?,?,?) ON CONFLICT(conversation_id) DO UPDATE SET document=excluded.document,updated_at=excluded.updated_at", id.String(), document, c.clock.Now().Millisecond())
	return sqliteError(err)
}
