package httpserver

import (
	"net/http"

	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/webapiproto"
)

func (e *Server) accounts() *accounts.Accounts {
	s := e.state.Services
	return accounts.New(s.Control, s.Hasher, s.Sessions, s.Limiter)
}

func (e *Server) setupStatus(w http.ResponseWriter, r *http.Request) error {
	needed, err := e.accounts().SetupNeeded(r.Context())
	if err != nil {
		return err
	}
	writeJSON(w, 200, webapiproto.SetupStatus{Needed: needed})
	return nil
}

func (e *Server) setup(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapiproto.DecodeSetupRequest)
	if err != nil {
		return err
	}
	result, err := e.accounts().Setup(r.Context(), request)
	if err != nil {
		return err
	}
	http.SetCookie(w, sessionCookie(result.Session.Token, result.Session.ExpiresAt, overHTTPS(r)))
	writeJSON(w, 201, webapiproto.Identity{User: result.User})
	return nil
}

func (e *Server) login(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapiproto.DecodeCredentials)
	if err != nil {
		return err
	}
	result, err := e.accounts().Login(r.Context(), request)
	if err != nil {
		return err
	}
	http.SetCookie(w, sessionCookie(result.Session.Token, result.Session.ExpiresAt, overHTTPS(r)))
	writeJSON(w, 200, webapiproto.Identity{User: result.User})
	return nil
}

func (e *Server) logout(w http.ResponseWriter, r *http.Request) error {
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

func (e *Server) me(w http.ResponseWriter, r *http.Request) error {
	writeJSON(w, 200, webapiproto.Identity{User: caller(r)})
	return nil
}

func (e *Server) nickname(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapiproto.DecodeNicknamePatch)
	if err != nil {
		return err
	}
	user, err := e.accounts().SetNickname(r.Context(), caller(r).ID, request)
	if err != nil {
		return err
	}
	e.state.Services.Sync.Mark(user.ID, pagesync.Part{Kind: pagesync.User})
	writeJSON(w, 200, webapiproto.Identity{User: user})
	return nil
}

func (e *Server) password(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapiproto.DecodePasswordChange)
	if err != nil {
		return err
	}
	if err := e.accounts().ChangePassword(r.Context(), caller(r).ID, request); err != nil {
		return err
	}
	w.WriteHeader(204)
	return nil
}

func (e *Server) users(w http.ResponseWriter, r *http.Request) error {
	users, err := e.accounts().Users(r.Context(), caller(r))
	if err != nil {
		return err
	}
	writeJSON(w, 200, webapiproto.Users{Users: users})
	return nil
}

func (e *Server) createUser(w http.ResponseWriter, r *http.Request) error {
	if !caller(r).Role.Outranks(webapiproto.RoleUser) {
		return accounts.ErrAdminRequired
	}
	request, err := decodeBody(r, webapiproto.DecodeCreateUser)
	if err != nil {
		return err
	}
	user, err := e.accounts().Create(r.Context(), caller(r), request)
	if err != nil {
		return err
	}
	writeJSON(w, 201, webapiproto.Identity{User: user})
	return nil
}

func (e *Server) resetPassword(w http.ResponseWriter, r *http.Request) error {
	if !caller(r).Role.Outranks(webapiproto.RoleUser) {
		return accounts.ErrAdminRequired
	}
	id, err := webapiproto.ParseUserID(r.PathValue("id"))
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
	request, err := decodeBody(r, webapiproto.DecodePasswordReset)
	if err != nil {
		return err
	}
	if err := e.accounts().ResetPassword(r.Context(), caller(r), id, request); err != nil {
		return err
	}
	w.WriteHeader(204)
	return nil
}
