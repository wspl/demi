package edge

import (
	"errors"
	"net/http"

	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/webapi"
)

func (e *Edge) startEmail(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeEmailChangeStart)
	if err != nil {
		return err
	}
	challenge, err := e.state.Services.Email.Start(r.Context(), caller(r).ID, request.Email, request.Password)
	switch {
	case errors.Is(err, database.ErrEmailTaken):
		return apiFailure(409, "email_taken", "That email is already in use")
	case errors.Is(err, database.ErrCoolingDown):
		return apiFailure(429, "too_many_attempts", "Wait a minute before requesting another code")
	case errors.Is(err, accounts.ErrMailUnavailable):
		return apiFailure(503, "mail_unavailable", "Email delivery is not configured")
	case errors.Is(err, accounts.ErrMailFailed):
		return apiFailure(503, "mail_failed", "Verification email could not be delivered; try again")
	case err != nil:
		return err
	}
	writeJSON(w, 202, webapi.EmailChangeStarted{Challenge: challenge})
	return nil
}

func (e *Edge) confirmEmail(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeEmailChangeConfirm)
	if err != nil {
		return err
	}
	user, err := e.state.Services.Email.Confirm(r.Context(), caller(r).ID, request.ID, request.Code)
	if errors.Is(err, database.ErrInvalidCode) {
		return apiFailure(400, "invalid_code", "Invalid or expired verification code")
	}
	if errors.Is(err, database.ErrEmailTaken) {
		return apiFailure(409, "email_taken", "That email is already in use")
	}
	if err != nil {
		return err
	}
	e.state.Services.Sync.Mark(user.ID, pagesync.Part{Kind: pagesync.User})
	writeJSON(w, 200, webapi.Identity{User: user})
	return nil
}
