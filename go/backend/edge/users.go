package edge

import (
	"context"
	"net/http"

	"github.com/wspl/demi/go/webapi"
)

// Account administration (web-api.md § Account API; product.md § User
// system): administrators list every account, create accounts of a role
// they outrank, and reset the password of an account they outrank. Nobody
// acts on a peer or on the master, and no account is deleted.

func (s *Server) listUsers(w http.ResponseWriter, r *http.Request) error {
	if _, err := administrator(r); err != nil {
		return err
	}
	users, err := s.services.Control.Users(r.Context())
	if err != nil {
		return err
	}
	writeJSON(w, http.StatusOK, webapi.Users{Users: users})
	return nil
}

func (s *Server) createUser(w http.ResponseWriter, r *http.Request) error {
	admin, err := administrator(r)
	if err != nil {
		return err
	}
	request, err := jsonBody[webapi.CreateUser](w, r)
	if err != nil {
		return err
	}
	role := request.Role.Role()
	if !admin.Role.Outranks(role) {
		return forbidden("Only the master creates admins")
	}
	user, err := s.createAccount(r.Context(), request.Email, request.Password, role)
	if err != nil {
		return err
	}
	writeJSON(w, http.StatusCreated, webapi.CreatedUser{User: user})
	return nil
}

// createAccount stores an account with password hashed; the address must be
// free. The caller has checked that its role may create role.
func (s *Server) createAccount(ctx context.Context, email webapi.EmailAddress, password webapi.Password, role webapi.Role) (webapi.UserDTO, error) {
	hash, err := s.services.Hasher.Hash(ctx, password)
	if err != nil {
		return webapi.UserDTO{}, err
	}
	user, err := s.services.Control.CreateUser(ctx, email, hash, role)
	if err != nil {
		return webapi.UserDTO{}, err
	}
	if user == nil {
		return webapi.UserDTO{}, newError(http.StatusConflict, webapi.ErrorCodeEmailTaken, "An account has that email")
	}
	return *user, nil
}

// resetPassword resets the password of an account the caller outranks. Who
// the account is and whether the caller may reset it are answered before
// the body.
func (s *Server) resetPassword(w http.ResponseWriter, r *http.Request) error {
	admin, err := administrator(r)
	if err != nil {
		return err
	}
	notFound := newError(http.StatusNotFound, webapi.ErrorCodeUserNotFound, "No such user")
	id, err := webapi.ParseUserID(r.PathValue("id"))
	if err != nil {
		return notFound
	}
	target, err := s.services.Control.Account(r.Context(), id)
	if err != nil {
		return err
	}
	if target == nil {
		return notFound
	}
	if !admin.Role.Outranks(target.User.Role) {
		return forbidden("A role acts on lower roles only")
	}
	reset, err := jsonBody[webapi.PasswordReset](w, r)
	if err != nil {
		return err
	}
	hash, err := s.services.Hasher.Hash(r.Context(), reset.Password)
	if err != nil {
		return err
	}
	if err := s.services.Control.SetPassword(r.Context(), target.User.ID, hash); err != nil {
		return err
	}
	w.WriteHeader(http.StatusNoContent)
	return nil
}
