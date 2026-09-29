package auth

import (
	"context"
	"crypto/rand"
	"fmt"
	"log/slog"
	"math/big"
	"time"

	"github.com/google/uuid"
	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/webapi"
)

type VerificationMail struct {
	Email     webapi.EmailAddress
	Code      string
	ExpiresAt core.Timestamp
}
type AccountMail interface {
	SendVerification(context.Context, VerificationMail) error
}
type StartRefusal string

const (
	MailUnavailable    StartRefusal = "mail_unavailable"
	InvalidCredentials StartRefusal = "invalid_credentials"
	EmailTaken         StartRefusal = "email_taken"
	CoolingDown        StartRefusal = "cooling_down"
	MailFailed         StartRefusal = "mail_failed"
)

type StartOutcome struct {
	Challenge *webapi.EmailChallengeDTO
	Refusal   StartRefusal
}
type EmailChanges struct {
	control *storage.Control
	hasher  *PasswordHasher
	mail    AccountMail
	key     []byte
}

func NewEmailChanges(control *storage.Control, hasher *PasswordHasher, mail AccountMail, key []byte) *EmailChanges {
	return &EmailChanges{control, hasher, mail, append([]byte(nil), key...)}
}
func (e *EmailChanges) Start(ctx context.Context, user webapi.UserID, email webapi.EmailAddress, password webapi.Password) (StartOutcome, error) {
	if e.mail == nil {
		return StartOutcome{Refusal: MailUnavailable}, nil
	}
	account, err := e.control.Account(ctx, user)
	if err != nil {
		return StartOutcome{}, err
	}
	if account == nil {
		return StartOutcome{Refusal: InvalidCredentials}, nil
	}
	valid, err := e.hasher.Verify(ctx, password, &account.Password)
	if err != nil {
		return StartOutcome{}, err
	}
	if !valid {
		return StartOutcome{Refusal: InvalidCredentials}, nil
	}
	taken, err := e.control.EmailInUse(ctx, email)
	if err != nil {
		return StartOutcome{}, err
	}
	if taken {
		return StartOutcome{Refusal: EmailTaken}, nil
	}
	number, err := rand.Int(rand.Reader, big.NewInt(1000000))
	if err != nil {
		return StartOutcome{}, err
	}
	code := fmt.Sprintf("%06d", number.Int64())
	id := uuid.NewString()
	expiry, err := e.control.IssueEmailChallenge(ctx, storage.ChallengeIssue{User: user, ID: id, Email: email, Password: account.Password, Code: storage.HashCode(e.key, id, code)}, storage.ChallengePolicy{Lifetime: 10 * time.Minute, Cooldown: time.Minute})
	if err != nil {
		return StartOutcome{}, err
	}
	if expiry == nil {
		return StartOutcome{Refusal: CoolingDown}, nil
	}
	if err = e.mail.SendVerification(ctx, VerificationMail{email, code, *expiry}); err != nil {
		slog.Warn("a verification mail was not delivered", "error", err)
		if err = e.control.DeleteEmailChallenge(ctx, user, id); err != nil {
			return StartOutcome{}, err
		}
		return StartOutcome{Refusal: MailFailed}, nil
	}
	return StartOutcome{Challenge: &webapi.EmailChallengeDTO{ID: id, Email: email, ExpiresAt: *expiry}}, nil
}
func (e *EmailChanges) Confirm(ctx context.Context, user webapi.UserID, id, code string) (storage.ChallengeOutcome, error) {
	return e.control.ConfirmEmailChallenge(ctx, user, id, storage.HashCode(e.key, id, code), 5)
}
