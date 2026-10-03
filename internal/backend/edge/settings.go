package edge

import (
	"net/http"

	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/webapi"
)

func (e *Edge) settings(w http.ResponseWriter, _ *http.Request) error {
	writeJSON(w, 200, webapi.Settings{Mode: e.state.Services.Mode})
	return nil
}
func (e *Edge) preferences(w http.ResponseWriter, r *http.Request) error {
	preferences, err := e.state.Services.Control.Preferences(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	writeJSON(w, 200, webapi.UserPreferences{Preferences: preferences})
	return nil
}
func (e *Edge) patchPreferences(w http.ResponseWriter, r *http.Request) error {
	patch, err := decodeBody(r, webapi.DecodePreferencesPatch)
	if err != nil {
		return err
	}
	if _, err := accounts.Check(patch); err != nil {
		return invalidBody(err)
	}
	preferences, err := accounts.NewPreferences(e.state.Services.Control).Patch(r.Context(), caller(r).ID, patch)
	if err != nil {
		return err
	}
	e.state.Services.Sync.Mark(caller(r).ID, pagesync.Part{Kind: pagesync.Preferences})
	writeJSON(w, 200, webapi.UserPreferences{Preferences: preferences})
	return nil
}
func (e *Edge) usage(w http.ResponseWriter, r *http.Request) error {
	totals, err := e.state.Services.Control.UsageTotals(r.Context(), caller(r).ID)
	if err != nil {
		return err
	}
	writeJSON(w, 200, webapi.UsageTotals{Totals: totals})
	return nil
}
func (e *Edge) instanceUsage(w http.ResponseWriter, r *http.Request) error {
	if e.state.Services.Mode != webapi.InstanceModeShared {
		return apiFailure(403, "forbidden", "The instance's usage is a shared instance's view")
	}
	if caller(r).Role == webapi.RoleUser {
		return apiFailure(403, "forbidden", "The instance's usage is for administrators")
	}
	users, err := e.state.Services.Control.InstanceUsage(r.Context())
	if err != nil {
		return err
	}
	writeJSON(w, 200, webapi.InstanceUsage{Users: users})
	return nil
}
