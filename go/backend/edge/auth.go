package edge

import (
	"net/http"

	"github.com/wspl/demi/go/backend"
	"github.com/wspl/demi/go/backend/auth"
	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/webapi"
)

// The routes of setup, sign-in and the caller's own account (web-api.md §
// Account API).

func (s *Server) setupStatus(w http.ResponseWriter, r *http.Request) error {
	users, err := s.services.Control.HasUsers(r.Context())
	if err != nil {
		return err
	}
	writeJSON(w, http.StatusOK, webapi.SetupStatus{Needed: !users})
	return nil
}

// setup creates the master account while the instance has none, and signs
// it in.
func (s *Server) setup(w http.ResponseWriter, r *http.Request) error {
	request, err := jsonBody[webapi.SetupRequest](w, r)
	if err != nil {
		return err
	}
	hash, err := s.services.Hasher.Hash(r.Context(), request.Password)
	if err != nil {
		return err
	}
	user, err := s.services.Control.CreateMaster(r.Context(), request.Email, hash)
	if err != nil {
		return err
	}
	if user == nil {
		return newError(http.StatusNotFound, webapi.ErrorCodeAlreadySetUp, "This instance has its master account")
	}
	return s.signIn(w, r, *user, http.StatusCreated)
}

func (s *Server) login(w http.ResponseWriter, r *http.Request) error {
	credentials, err := jsonBody[webapi.Credentials](w, r)
	if err != nil {
		return err
	}
	email := credentials.Email
	if s.services.Limiter.Locked(email) {
		return newError(http.StatusTooManyRequests, webapi.ErrorCodeTooManyAttempts, "Too many failed logins; try again in a minute")
	}
	account, err := s.services.Control.AccountByEmail(r.Context(), email)
	if err != nil {
		return err
	}
	var stored *storage.PasswordHash
	if account != nil {
		stored = &account.Password
	}
	verified, err := s.services.Hasher.Verify(r.Context(), credentials.Password, stored)
	if err != nil {
		return err
	}
	if !verified {
		s.services.Limiter.Failed(email)
		return newError(http.StatusUnauthorized, webapi.ErrorCodeInvalidCredentials, "Wrong email or password")
	}
	s.services.Limiter.Succeeded(email)
	return s.signIn(w, r, account.User, http.StatusOK)
}

// signIn opens a session for user, sets its cookie, and answers the user.
func (s *Server) signIn(w http.ResponseWriter, r *http.Request, user webapi.UserDTO, status int) error {
	opened, err := s.services.Sessions.Open(r.Context(), user.ID)
	if err != nil {
		return err
	}
	http.SetCookie(w, sessionCookieFor(opened.Token, opened.ExpiresAt, overHTTPS(r)))
	writeJSON(w, status, webapi.Identity{User: user})
	return nil
}

// logout ends the session the cookie names, with the synchronization
// channels that opened with it, and clears the cookie.
func (s *Server) logout(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	if cookie, err := r.Cookie(sessionCookie); err == nil {
		if err := s.services.Sessions.Close(r.Context(), cookie.Value); err != nil {
			return err
		}
		s.services.Sync.EndSession(user.ID, storage.HashToken(cookie.Value))
	}
	http.SetCookie(w, clearedCookie(overHTTPS(r)))
	w.WriteHeader(http.StatusNoContent)
	return nil
}

func (s *Server) me(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	writeJSON(w, http.StatusOK, webapi.Identity{User: user})
	return nil
}

func (s *Server) setNickname(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	patch, err := jsonBody[webapi.NicknamePatch](w, r)
	if err != nil {
		return err
	}
	changed, err := s.services.Control.SetNickname(r.Context(), user.ID, patch.Nickname.String())
	if err != nil {
		return err
	}
	if changed == nil {
		return unauthenticated()
	}
	s.services.Sync.Mark(changed.ID, backend.SyncUser)
	writeJSON(w, http.StatusOK, webapi.Identity{User: *changed})
	return nil
}

// changePassword changes the caller's password when the current one checks.
func (s *Server) changePassword(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	change, err := jsonBody[webapi.PasswordChange](w, r)
	if err != nil {
		return err
	}
	account, err := s.services.Control.Account(r.Context(), user.ID)
	if err != nil {
		return err
	}
	var stored *storage.PasswordHash
	if account != nil {
		stored = &account.Password
	}
	verified, err := s.services.Hasher.Verify(r.Context(), change.Current, stored)
	if err != nil {
		return err
	}
	if !verified {
		return newError(http.StatusUnauthorized, webapi.ErrorCodeInvalidCredentials, "Current password is wrong")
	}
	hash, err := s.services.Hasher.Hash(r.Context(), change.Next)
	if err != nil {
		return err
	}
	if err := s.services.Control.SetPassword(r.Context(), user.ID, hash); err != nil {
		return err
	}
	w.WriteHeader(http.StatusNoContent)
	return nil
}

func (s *Server) startEmailChange(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	start, err := jsonBody[webapi.EmailChangeStart](w, r)
	if err != nil {
		return err
	}
	outcome, err := s.services.Email.Start(r.Context(), user.ID, start.Email, start.Password)
	if err != nil {
		return err
	}
	if outcome.Challenge == nil {
		return emailChangeRefused(outcome.Refusal)
	}
	writeJSON(w, http.StatusAccepted, webapi.EmailChangeStarted{Challenge: *outcome.Challenge})
	return nil
}

func emailChangeRefused(refusal auth.StartRefusal) *apiError {
	switch refusal {
	case auth.InvalidCredentials:
		return newError(http.StatusUnauthorized, webapi.ErrorCodeInvalidCredentials, "Current password is wrong")
	case auth.EmailTaken:
		return emailTaken()
	case auth.CoolingDown:
		return newError(http.StatusTooManyRequests, webapi.ErrorCodeTooManyAttempts, "Wait a minute before requesting another code")
	case auth.MailUnavailable:
		return newError(http.StatusServiceUnavailable, webapi.ErrorCodeMailUnavailable, "Email delivery is not configured")
	}
	return newError(http.StatusServiceUnavailable, webapi.ErrorCodeMailFailed, "Verification email could not be delivered; try again")
}

func emailTaken() *apiError {
	return newError(http.StatusConflict, webapi.ErrorCodeEmailTaken, "That email is already in use")
}

func (s *Server) confirmEmailChange(w http.ResponseWriter, r *http.Request) error {
	user, err := caller(r)
	if err != nil {
		return err
	}
	confirm, err := jsonBody[webapi.EmailChangeConfirm](w, r)
	if err != nil {
		return err
	}
	outcome, err := s.services.Email.Confirm(r.Context(), user.ID, confirm.ID, confirm.Code)
	if err != nil {
		return err
	}
	switch outcome.Kind {
	case storage.ChallengeChanged:
		s.services.Sync.Mark(outcome.User.ID, backend.SyncUser)
		writeJSON(w, http.StatusOK, webapi.Identity{User: *outcome.User})
		return nil
	case storage.ChallengeEmailTaken:
		return emailTaken()
	}
	return newError(http.StatusBadRequest, webapi.ErrorCodeInvalidCode, "Invalid or expired verification code")
}
