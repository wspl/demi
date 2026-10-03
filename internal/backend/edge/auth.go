package edge

import (
	"net/http"

	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/webapi"
)

func (e *Edge) accounts() *accounts.Accounts {
	s := e.state.Services
	return accounts.NewAccounts(s.Control, s.Hasher, s.Sessions, s.Limiter)
}

func (e *Edge) setupStatus(w http.ResponseWriter, r *http.Request) error {
	needed, err := e.accounts().SetupNeeded(r.Context())
	if err != nil {
		return err
	}
	writeJSON(w, 200, webapi.SetupStatus{Needed: needed})
	return nil
}

func (e *Edge) setup(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeSetupRequest)
	if err != nil {
		return err
	}
	result, err := e.accounts().Setup(r.Context(), request)
	if err != nil {
		return err
	}
	http.SetCookie(w, sessionCookie(result.Session.Token, result.Session.ExpiresAt, overHTTPS(r)))
	writeJSON(w, 201, webapi.Identity{User: result.User})
	return nil
}

func (e *Edge) login(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeCredentials)
	if err != nil {
		return err
	}
	result, err := e.accounts().Login(r.Context(), request)
	if err != nil {
		return err
	}
	http.SetCookie(w, sessionCookie(result.Session.Token, result.Session.ExpiresAt, overHTTPS(r)))
	writeJSON(w, 200, webapi.Identity{User: result.User})
	return nil
}

func (e *Edge) logout(w http.ResponseWriter, r *http.Request) error {
	if cookie, err := r.Cookie("demi_session"); err == nil {
		if err := e.state.Services.Sessions.Close(r.Context(), cookie.Value); err != nil {
			return err
		}
		e.state.Services.Sync.EndSession(caller(r).ID, database.HashToken(cookie.Value))
	}
	removeCookie(w, r)
	w.WriteHeader(204)
	return nil
}

func (e *Edge) me(w http.ResponseWriter, r *http.Request) error {
	writeJSON(w, 200, webapi.Identity{User: caller(r)})
	return nil
}

func (e *Edge) nickname(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeNicknamePatch)
	if err != nil {
		return err
	}
	user, err := e.accounts().SetNickname(r.Context(), caller(r).ID, request)
	if err != nil {
		return err
	}
	e.state.Services.Sync.Mark(user.ID, pagesync.Part{Kind: pagesync.User})
	writeJSON(w, 200, webapi.Identity{User: user})
	return nil
}

func (e *Edge) password(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodePasswordChange)
	if err != nil {
		return err
	}
	if err := e.accounts().ChangePassword(r.Context(), caller(r).ID, request); err != nil {
		return err
	}
	w.WriteHeader(204)
	return nil
}

func (e *Edge) users(w http.ResponseWriter, r *http.Request) error {
	users, err := e.accounts().Users(r.Context(), caller(r))
	if err != nil {
		return err
	}
	writeJSON(w, 200, webapi.Users{Users: users})
	return nil
}

func (e *Edge) createUser(w http.ResponseWriter, r *http.Request) error {
	if !caller(r).Role.Outranks(webapi.RoleUser) {
		return accounts.ErrAdminRequired
	}
	request, err := decodeBody(r, webapi.DecodeCreateUser)
	if err != nil {
		return err
	}
	user, err := e.accounts().Create(r.Context(), caller(r), request)
	if err != nil {
		return err
	}
	writeJSON(w, 201, webapi.Identity{User: user})
	return nil
}

func (e *Edge) resetPassword(w http.ResponseWriter, r *http.Request) error {
	if !caller(r).Role.Outranks(webapi.RoleUser) {
		return accounts.ErrAdminRequired
	}
	id, err := webapi.ParseUserID(r.PathValue("id"))
	if err != nil {
		return accounts.ErrUserNotFound
	}
	account, found, err := e.state.Services.Control.Account(r.Context(), id)
	if err != nil {
		return err
	}
	if !found {
		return accounts.ErrUserNotFound
	}
	if !caller(r).Role.Outranks(account.User.Role) {
		return accounts.ErrLowerRolesOnly
	}
	request, err := decodeBody(r, webapi.DecodePasswordReset)
	if err != nil {
		return err
	}
	if err := e.accounts().ResetPassword(r.Context(), caller(r), id, request); err != nil {
		return err
	}
	w.WriteHeader(204)
	return nil
}
