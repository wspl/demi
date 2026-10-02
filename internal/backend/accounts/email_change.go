package accounts

import (
	"context"
	"crypto/rand"
	"fmt"
	"log/slog"
	"math/big"
	"time"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// AccountMail delivers verification codes. A deployment supplies it; tests
// capture the mail instead of sending it.
type AccountMail interface {
	SendVerification(context.Context, VerificationMail) error
}

// VerificationMail is a verification code on its way to the address it proves.
type VerificationMail struct {
	Email     webapi.EmailAddress
	Code      string
	ExpiresAt core.Timestamp
}

// CodeKey is the key email-change codes are hashed under, supplied by assembly.
type CodeKey [32]byte

// NewCodeKey wraps the instance-derived email-change key.
func NewCodeKey(key [32]byte) CodeKey { return CodeKey(key) }

// StartRefusal is why an email-change start was refused.
type StartRefusal string

// Reasons an email change may not start.
const (
	MailUnavailable    StartRefusal = "mail_unavailable"
	InvalidCredentials StartRefusal = "invalid_credentials"
	EmailTaken         StartRefusal = "email_taken"
	CoolingDown        StartRefusal = "cooling_down"
	MailFailed         StartRefusal = "mail_failed"
)

// StartOutcome either issues a challenge or refuses it.
//
//sumtype:decl
type StartOutcome interface{ startOutcome() }

// StartIssued contains the challenge sent to the new address.
type StartIssued struct{ Challenge webapi.EmailChallengeDTO }

func (*StartIssued) startOutcome() {}

// StartRefused contains the reason no challenge was sent.
type StartRefused struct{ Reason StartRefusal }

func (*StartRefused) startOutcome() {}

// EmailStore is the control database's email-change boundary.
type EmailStore interface {
	Account(context.Context, webapi.UserID) (*database.Account, error)
	EmailInUse(context.Context, webapi.EmailAddress) (bool, error)
	IssueEmailChallenge(context.Context, database.ChallengeIssue, database.ChallengePolicy) (*core.Timestamp, error)
	DeleteEmailChallenge(context.Context, webapi.UserID, string) error
	ConfirmEmailChallenge(context.Context, webapi.UserID, string, database.CodeHash, uint32) (database.ChallengeOutcome, error)
}

// PasswordVerifier checks current account credentials.
type PasswordVerifier interface {
	Verify(context.Context, webapi.Password, *database.PasswordHash) (bool, error)
}

// EmailChanges authenticates and delivers challenges; storage consumes them.
type EmailChanges struct {
	control EmailStore
	hasher  PasswordVerifier
	mail    AccountMail
	key     CodeKey
}

// NewEmailChanges returns the email-change service. Nil mail disables delivery.
func NewEmailChanges(control EmailStore, hasher PasswordVerifier, mail AccountMail, key CodeKey) *EmailChanges {
	return &EmailChanges{control: control, hasher: hasher, mail: mail, key: key}
}

func challengePolicy() database.ChallengePolicy {
	return database.ChallengePolicy{Lifetime: 10 * time.Minute, Cooldown: time.Minute, Attempts: 5}
}

// Start checks the current password and sends a code to email.
func (e *EmailChanges) Start(ctx context.Context, user webapi.UserID, email webapi.EmailAddress, password webapi.Password) (StartOutcome, error) {
	if e.mail == nil {
		return &StartRefused{Reason: MailUnavailable}, nil
	}
	account, err := e.control.Account(ctx, user)
	if err != nil {
		return nil, err
	}
	if account == nil {
		return &StartRefused{Reason: InvalidCredentials}, nil
	}
	verified, err := e.hasher.Verify(ctx, password, &account.PasswordHash)
	if err != nil {
		return nil, err
	}
	if !verified {
		return &StartRefused{Reason: InvalidCredentials}, nil
	}
	taken, err := e.control.EmailInUse(ctx, email)
	if err != nil {
		return nil, err
	}
	if taken {
		return &StartRefused{Reason: EmailTaken}, nil
	}
	id, err := uuid.NewRandom()
	if err != nil {
		return nil, fmt.Errorf("generate email challenge: %w", err)
	}
	number, err := rand.Int(rand.Reader, big.NewInt(1000000))
	if err != nil {
		return nil, fmt.Errorf("generate verification code: %w", err)
	}
	code := fmt.Sprintf("%06d", number.Int64())
	issue := database.ChallengeIssue{User: user, ID: id.String(), Email: email, PasswordHash: account.PasswordHash, CodeHash: database.HashCode(e.key[:], id.String(), code)}
	expires, err := e.control.IssueEmailChallenge(ctx, issue, challengePolicy())
	if err != nil {
		return nil, err
	}
	if expires == nil {
		return &StartRefused{Reason: CoolingDown}, nil
	}
	mail := VerificationMail{Email: email, Code: code, ExpiresAt: *expires}
	if err := e.mail.SendVerification(ctx, mail); err != nil {
		slog.Warn("a verification mail was not delivered", "error", err)
		// Failed delivery must release the challenge and cooldown even if the
		// requester left. The caller owns and waits for this cleanup.
		if err := e.control.DeleteEmailChallenge(context.WithoutCancel(ctx), user, issue.ID); err != nil {
			return nil, err
		}
		return &StartRefused{Reason: MailFailed}, nil
	}
	return &StartIssued{Challenge: webapi.EmailChallengeDTO{ID: issue.ID, Email: email, ExpiresAt: *expires}}, nil
}

// Confirm changes the address when code is the one the challenge sent.
func (e *EmailChanges) Confirm(ctx context.Context, user webapi.UserID, challenge, code string) (database.ChallengeOutcome, error) {
	return e.control.ConfirmEmailChallenge(ctx, user, challenge, database.HashCode(e.key[:], challenge, code), challengePolicy().Attempts)
}

// Format keeps the email-change key out of diagnostics.
func (CodeKey) Format(state fmt.State, _ rune) {
	// fmt owns the destination and reports write errors to its caller.
	_, _ = state.Write([]byte("CodeKey(..)"))
}
