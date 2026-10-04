package accounts

import (
	"context"
	"crypto/rand"
	"errors"
	"fmt"
	"log/slog"
	"math/big"
	"time"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// AccountMail delivers verification codes. A deployment supplies it; tests
// capture the mail instead of sending it.
type AccountMail interface {
	SendVerification(context.Context, VerificationMail) error
}

// VerificationMail is a verification code on its way to the address it proves.
type VerificationMail struct {
	Email     webapiproto.EmailAddress
	Code      string
	ExpiresAt types.Timestamp
}

// CodeKey is the key email-change codes are hashed under, supplied by assembly.
type CodeKey [32]byte

// NewCodeKey wraps the instance-derived email-change key.
func NewCodeKey(key [32]byte) CodeKey {
	return CodeKey(key)
}

var (
	// ErrMailUnavailable means no mail delivery service is configured.
	ErrMailUnavailable = errors.New("no mail delivery is configured")
	// ErrMailFailed means the verification mail could not be delivered.
	ErrMailFailed = errors.New("the verification mail was not delivered")
)

// EmailStore is the control database's email-change boundary.
type EmailStore interface {
	Account(context.Context, webapiproto.UserID) (database.Account, bool, error)
	EmailInUse(context.Context, webapiproto.EmailAddress) (bool, error)
	IssueEmailChallenge(context.Context, database.ChallengeIssue, database.ChallengePolicy) (types.Timestamp, error)
	DeleteEmailChallenge(context.Context, webapiproto.UserID, string) error
	ConfirmEmailChallenge(
		context.Context,
		webapiproto.UserID,
		string,
		database.CodeHash,
		uint32,
	) (webapiproto.UserDTO, error)
}

// PasswordVerifier checks current account credentials.
type PasswordVerifier interface {
	Verify(context.Context, webapiproto.Password, *database.PasswordHash) (bool, error)
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
func (e *EmailChanges) Start(
	ctx context.Context,
	user webapiproto.UserID,
	email webapiproto.EmailAddress,
	password webapiproto.Password,
) (webapiproto.EmailChallengeDTO, error) {
	if e.mail == nil {
		return webapiproto.EmailChallengeDTO{}, ErrMailUnavailable
	}
	account, found, err := e.control.Account(ctx, user)
	if err != nil {
		return webapiproto.EmailChallengeDTO{}, err
	}
	if !found {
		return webapiproto.EmailChallengeDTO{}, ErrCurrentPassword
	}
	verified, err := e.hasher.Verify(ctx, password, &account.PasswordHash)
	if err != nil {
		return webapiproto.EmailChallengeDTO{}, err
	}
	if !verified {
		return webapiproto.EmailChallengeDTO{}, ErrCurrentPassword
	}
	taken, err := e.control.EmailInUse(ctx, email)
	if err != nil {
		return webapiproto.EmailChallengeDTO{}, err
	}
	if taken {
		return webapiproto.EmailChallengeDTO{}, database.ErrEmailTaken
	}
	return e.issueChallenge(ctx, user, email, account.PasswordHash)
}

// Confirm changes the address when code is the one the challenge sent.
func (e *EmailChanges) Confirm(
	ctx context.Context,
	user webapiproto.UserID,
	challenge, code string,
) (webapiproto.UserDTO, error) {
	return e.control.ConfirmEmailChallenge(
		ctx,
		user,
		challenge,
		database.HashCode(e.key[:], challenge, code),
		challengePolicy().Attempts,
	)
}

// Format keeps the email-change key out of diagnostics.
func (CodeKey) Format(state fmt.State, _ rune) {
	// fmt owns the destination and reports write errors to its caller.
	_, _ = state.Write([]byte("CodeKey(..)"))
}

func (e *EmailChanges) issueChallenge(
	ctx context.Context,
	user webapiproto.UserID,
	email webapiproto.EmailAddress,
	passwordHash database.PasswordHash,
) (webapiproto.EmailChallengeDTO, error) {
	id, err := uuid.NewRandom()
	if err != nil {
		return webapiproto.EmailChallengeDTO{}, fmt.Errorf("generate email challenge: %w", err)
	}
	number, err := rand.Int(rand.Reader, big.NewInt(1000000))
	if err != nil {
		return webapiproto.EmailChallengeDTO{}, fmt.Errorf("generate verification code: %w", err)
	}
	code := fmt.Sprintf("%06d", number.Int64())
	issue := database.ChallengeIssue{
		User:         user,
		ID:           id.String(),
		Email:        email,
		PasswordHash: passwordHash,
		CodeHash:     database.HashCode(e.key[:], id.String(), code),
	}
	expires, err := e.control.IssueEmailChallenge(ctx, issue, challengePolicy())
	if err != nil {
		return webapiproto.EmailChallengeDTO{}, err
	}
	mail := VerificationMail{Email: email, Code: code, ExpiresAt: expires}
	if err := e.mail.SendVerification(ctx, mail); err != nil {
		slog.Warn("a verification mail was not delivered", "error", err)
		// Failed delivery must release the challenge and cooldown even if the
		// requester left. The caller owns and waits for this cleanup.
		if err := e.control.DeleteEmailChallenge(context.WithoutCancel(ctx), user, issue.ID); err != nil {
			return webapiproto.EmailChallengeDTO{}, err
		}
		return webapiproto.EmailChallengeDTO{}, ErrMailFailed
	}
	return webapiproto.EmailChallengeDTO{ID: issue.ID, Email: email, ExpiresAt: expires}, nil
}
