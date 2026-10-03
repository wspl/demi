package edge

import (
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
	result, err := e.state.Services.Email.Start(r.Context(), caller(r).ID, request.Email, request.Password)
	if err != nil {
		return err
	}
	switch result := result.(type) {
	case *accounts.StartIssued:
		writeJSON(w, 202, webapi.EmailChangeStarted{Challenge: result.Challenge})
		return nil
	case *accounts.StartRefused:
		switch result.Reason {
		case accounts.InvalidCredentials:
			return accounts.ErrCurrentPassword
		case accounts.EmailTaken:
			return apiFailure(409, "email_taken", "That email is already in use")
		case accounts.CoolingDown:
			return apiFailure(429, "too_many_attempts", "Wait a minute before requesting another code")
		case accounts.MailUnavailable:
			return apiFailure(503, "mail_unavailable", "Email delivery is not configured")
		case accounts.MailFailed:
			return apiFailure(503, "mail_failed", "Verification email could not be delivered; try again")
		}
	}
	return nil
}

func (e *Edge) confirmEmail(w http.ResponseWriter, r *http.Request) error {
	request, err := decodeBody(r, webapi.DecodeEmailChangeConfirm)
	if err != nil {
		return err
	}
	result, err := e.state.Services.Email.Confirm(r.Context(), caller(r).ID, request.ID, request.Code)
	if err != nil {
		return err
	}
	switch result := result.(type) {
	case *database.ChallengeChanged:
		e.state.Services.Sync.Mark(result.User.ID, pagesync.Part{Kind: pagesync.User})
		writeJSON(w, 200, webapi.Identity{User: result.User})
	case *database.ChallengeInvalidCode:
		return apiFailure(400, "invalid_code", "Invalid or expired verification code")
	case *database.ChallengeEmailTaken:
		return apiFailure(409, "email_taken", "That email is already in use")
	}
	return nil
}
