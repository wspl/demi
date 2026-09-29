package edge

import (
	"errors"
	"net/http"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/webapi"
)

// A subscription entry's accounts and device logins (web-api.md §
// Subscription accounts): setup-token imports, the account list and its
// active account, removal, and logins into a new or an existing entry.
// Changing accounts is the configuring user's, like every other change of
// an entry; tokens are never returned.

// importSetupToken creates the caller's Claude Code entry from a setup
// token.
func (s *Server) importSetupToken(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	request, err := jsonBody[webapi.SetupTokenImport](w, r)
	if err != nil {
		return err
	}
	if err := s.configures(user); err != nil {
		return err
	}
	owner, err := s.services.Vault.OwnerFor(r.Context(), user.ID)
	if err != nil {
		return err
	}
	entry, err := s.services.Assembly.ImportSetupToken(r.Context(), owner, string(request.Label), string(request.Token))
	if err != nil {
		return err
	}
	writeJSON(w, http.StatusCreated, webapi.ProviderAnswer{Provider: entry.DTO()})
	return nil
}

// listAccounts is the entry's accounts; a user who only infers sees none.
func (s *Server) listAccounts(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	entry, err := s.scoped(r, user)
	if err != nil {
		return err
	}
	accounts, err := s.services.Assembly.ListAccounts(r.Context(), entry, s.services.Vault.Configures(user))
	if err != nil {
		return err
	}
	writeJSON(w, http.StatusOK, accounts)
	return nil
}

// addToken adds another account to the entry from a setup token.
func (s *Server) addToken(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	request, err := jsonBody[webapi.AddToken](w, r)
	if err != nil {
		return err
	}
	entry, err := s.configuredEntry(r, user)
	if err != nil {
		return err
	}
	release, err := s.reserve(entry)
	if err != nil {
		return err
	}
	defer release()
	account, err := s.services.Assembly.AddToken(r.Context(), entry, string(request.Token))
	if err != nil {
		return err
	}
	writeJSON(w, http.StatusCreated, webapi.AddedAccount{Account: account})
	return nil
}

// activateAccount selects the account the entry infers with.
func (s *Server) activateAccount(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	request, err := jsonBody[webapi.ActivateAccount](w, r)
	if err != nil {
		return err
	}
	entry, err := s.configuredEntry(r, user)
	if err != nil {
		return err
	}
	release, err := s.reserve(entry)
	if err != nil {
		return err
	}
	defer release()
	active, err := s.services.Assembly.ActivateAccount(r.Context(), entry, request.CredentialID)
	if err != nil {
		return err
	}
	writeJSON(w, http.StatusOK, webapi.ActiveAccount{Active: active})
	return nil
}

// removeAccount removes an account other than the active one.
func (s *Server) removeAccount(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	entry, err := s.configuredEntry(r, user)
	if err != nil {
		return err
	}
	release, err := s.reserve(entry)
	if err != nil {
		return err
	}
	defer release()
	account, err := webapi.ParseCredentialID(r.PathValue("credential"))
	if err != nil {
		return accountNotFound()
	}
	if err := s.services.Assembly.RemoveAccount(r.Context(), entry, account); err != nil {
		return err
	}
	w.WriteHeader(http.StatusNoContent)
	return nil
}

// loginStarted answers a login that started.
func loginStarted(w http.ResponseWriter, id webapi.LoginID) {
	login := webapi.StartedLogin{ID: id, Status: webapi.PendingStatusPending}
	writeJSON(w, http.StatusAccepted, webapi.LoginStarted{Login: login})
}

// loginRefused is the answer of a login that did not start: a family the
// backend does not register is the request's error.
func loginRefused(err error) error {
	var unknown *backend.UnknownFamilyError
	if errors.As(err, &unknown) {
		return newError(http.StatusBadRequest, webapi.ErrorCodeUnknownProviderType, err.Error())
	}
	return err
}

// startLogin starts a device login into a new entry of the family the body
// names.
func (s *Server) startLogin(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	request, err := jsonBody[webapi.SubscriptionLogin](w, r)
	if err != nil {
		return err
	}
	if err := s.configures(user); err != nil {
		return err
	}
	family := request.ProviderType
	label := family + " subscription"
	if request.Label != nil {
		label = string(*request.Label)
	}
	owner, err := s.services.Vault.OwnerFor(r.Context(), user.ID)
	if err != nil {
		return err
	}
	id, err := s.services.Logins.Start(r.Context(), owner, user.ID, family, label, nil)
	if err != nil {
		return loginRefused(err)
	}
	loginStarted(w, id)
	return nil
}

// loginInto starts a device login of another account into the entry,
// which the login holds until it ends.
func (s *Server) loginInto(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	entry, err := s.configuredEntry(r, user)
	if err != nil {
		return err
	}
	if entry.Kind() != webapi.CredentialKindSubscription {
		return newError(http.StatusBadRequest, webapi.ErrorCodeAccountsUnsupported, "This provider does not use subscription accounts")
	}
	id, err := s.services.Logins.Start(r.Context(), entry.Owner, user.ID, entry.Family, entry.Label, &entry)
	if err != nil {
		return loginRefused(err)
	}
	loginStarted(w, id)
	return nil
}

func loginNotFound() *apiError {
	return newError(http.StatusNotFound, webapi.ErrorCodeLoginNotFound, "No such login")
}

// loginState is where the caller's login is.
func (s *Server) loginState(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	id, err := webapi.ParseLoginID(r.PathValue("id"))
	if err != nil {
		return loginNotFound()
	}
	login, ok := s.services.Logins.State(id, user.ID)
	if !ok {
		return loginNotFound()
	}
	writeJSON(w, http.StatusOK, webapi.LoginAnswer{Login: login})
	return nil
}

// cancelLogin cancels the caller's login, which stops at once, even
// between polls.
func (s *Server) cancelLogin(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	if err := s.configures(user); err != nil {
		return err
	}
	id, err := webapi.ParseLoginID(r.PathValue("id"))
	if err != nil {
		return loginNotFound()
	}
	// The login is cancelled once Cancel says so: a wait that ended early
	// means the client went away, and nobody reads the answer.
	cancelled, _ := s.services.Logins.Cancel(r.Context(), id, user.ID)
	if !cancelled {
		return loginNotFound()
	}
	w.WriteHeader(http.StatusNoContent)
	return nil
}
