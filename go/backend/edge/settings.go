package edge

import (
	"net/http"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/webapi"
)

// The instance's settings and the caller's preferences (web-api.md § User
// preferences). Preferences belong to the signed-in user alone, in both
// instance modes.

func (s *Server) settings(w http.ResponseWriter, r *http.Request) error {
	writeJSON(w, http.StatusOK, webapi.Settings{Mode: s.services.Mode})
	return nil
}

func (s *Server) preferences(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	preferences, err := s.services.Control.Preferences(r.Context(), user.ID)
	if err != nil {
		return err
	}
	writeJSON(w, http.StatusOK, webapi.UserPreferences{Preferences: preferences})
	return nil
}

// patchPreferences merges the patch into the caller's preferences; a
// reported locale must name a known time zone and well-formed language
// tags.
func (s *Server) patchPreferences(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	patch, err := jsonBody[webapi.PreferencesPatch](w, r)
	if err != nil {
		return err
	}
	checked, err := storage.CheckPreferences(patch, backend.CanonicalLocale)
	if err != nil {
		return invalidBody(err.Error())
	}
	preferences, err := s.services.Control.PatchPreferences(r.Context(), user.ID, checked)
	if err != nil {
		return err
	}
	s.services.Sync.Mark(user.ID, backend.SyncPreferences)
	writeJSON(w, http.StatusOK, webapi.UserPreferences{Preferences: preferences})
	return nil
}

// The usage ledger's totals (usage-and-quota.md § Usage ledger): each
// user's own, and on a shared instance every account's for an
// administrator.

func (s *Server) usageTotals(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	totals, err := s.services.Control.UsageTotals(r.Context(), user.ID)
	if err != nil {
		return err
	}
	writeJSON(w, http.StatusOK, webapi.UsageTotals{Totals: totals})
	return nil
}

func (s *Server) instanceUsage(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	if s.services.Mode != webapi.InstanceModeShared {
		return forbidden("The instance's usage is a shared instance's view")
	}
	if user.Role == webapi.RoleUser {
		return forbidden("The instance's usage is for administrators")
	}
	users, err := s.services.Control.InstanceUsage(r.Context())
	if err != nil {
		return err
	}
	writeJSON(w, http.StatusOK, webapi.InstanceUsage{Users: users})
	return nil
}

// models is GET /api/models (models.md § What the browser receives): the
// catalog of every entry the caller infers with, each model with the
// selection the backend builds from it and each entry with its health. It
// wakes no Cloud and runs no model; refresh=true waits for a shared forced
// refresh.
func (s *Server) models(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	refresh, err := refreshQuery(r)
	if err != nil {
		return err
	}
	owner, err := s.services.Vault.OwnerFor(r.Context(), user.ID)
	if err != nil {
		return err
	}
	entries, err := s.services.Vault.Entries(r.Context(), owner)
	if err != nil {
		return err
	}
	providers := s.services.Assembly.ModelCatalog(r.Context(), entries, refresh)
	writeJSON(w, http.StatusOK, webapi.ModelCatalog{Providers: providers})
	return nil
}

// refreshQuery is the request's refresh query parameter: true or false,
// false when absent.
func refreshQuery(r *http.Request) (bool, error) {
	var refresh webapi.StrictBool
	if values, ok := r.URL.Query()["refresh"]; ok {
		if err := refresh.UnmarshalText([]byte(values[0])); err != nil {
			return false, invalidQuery("refresh: " + err.Error())
		}
	}
	return bool(refresh), nil
}
