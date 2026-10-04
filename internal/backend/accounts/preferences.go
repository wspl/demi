package accounts

import (
	"context"

	"github.com/wspl/demi/internal/webapiproto"
)

// PreferenceStore reads preferences and atomically applies serializable patches.
// The database owns the only merge; concurrent changes to other fields survive.
type PreferenceStore interface {
	Preferences(context.Context, webapiproto.UserID) (webapiproto.Preferences, error)
	PatchPreferences(context.Context, webapiproto.UserID, webapiproto.PreferencesPatch) (webapiproto.Preferences, error)
}

// Preferences reads and changes only the authenticated caller's preferences.
// The edge supplies the authenticated user; there is no separate target user.
type Preferences struct{ control PreferenceStore }

// NewPreferences returns the per-user preference service.
func NewPreferences(control PreferenceStore) *Preferences { return &Preferences{control: control} }

// Read returns the authenticated user's saved overrides.
func (p *Preferences) Read(ctx context.Context, caller webapiproto.UserID) (webapiproto.Preferences, error) {
	return p.control.Preferences(ctx, caller)
}

// Patch validates the locale and delegates one atomic patch to storage.
func (p *Preferences) Patch(
	ctx context.Context,
	caller webapiproto.UserID,
	patch webapiproto.PreferencesPatch,
) (webapiproto.Preferences, error) {
	checked, err := Check(patch)
	if err != nil {
		return webapiproto.Preferences{}, err
	}
	return p.control.PatchPreferences(ctx, caller, checked.patch)
}
