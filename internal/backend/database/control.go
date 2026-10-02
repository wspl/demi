package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// Close closes the database; operations after it fail with ErrClosed.
func (c *ControlService) Close(ctx context.Context) error { panic("not written: b-database") }

// HasUsers reports whether the instance has any accounts.
func (c *ControlService) HasUsers(ctx context.Context) (bool, error) {
	panic("not written: b-database")
}

// CreateMaster returns the instance's first account, created only while there is no account
// at all; nil once setup has run.
func (c *ControlService) CreateMaster(ctx context.Context, email webapi.EmailAddress, passwordHash PasswordHash) (*webapi.UserDTO, error) {
	panic("not written: b-database")
}

// AccountByEmail returns the login lookup.
func (c *ControlService) AccountByEmail(ctx context.Context, email webapi.EmailAddress) (*Account, error) {
	panic("not written: b-database")
}

// Account returns the account of user, or nil when absent.
func (c *ControlService) Account(ctx context.Context, user webapi.UserID) (*Account, error) {
	panic("not written: b-database")
}

// Users returns every account, in the order they were created.
func (c *ControlService) Users(ctx context.Context) ([]webapi.UserDTO, error) {
	panic("not written: b-database")
}

// CreateUser returns a new account of `role`; nil, writing nothing, when an account has
// the address already.
func (c *ControlService) CreateUser(ctx context.Context, email webapi.EmailAddress, passwordHash PasswordHash, role webapi.Role) (*webapi.UserDTO, error) {
	panic("not written: b-database")
}

// EmailInUse reports whether an account already has email.
func (c *ControlService) EmailInUse(ctx context.Context, email webapi.EmailAddress) (bool, error) {
	panic("not written: b-database")
}

// SetNickname sets the nickname and answers the account as it now is.
func (c *ControlService) SetNickname(ctx context.Context, user webapi.UserID, nickname string) (*webapi.UserDTO, error) {
	panic("not written: b-database")
}

// SetPassword replaces the account password hash.
func (c *ControlService) SetPassword(ctx context.Context, user webapi.UserID, passwordHash PasswordHash) error {
	panic("not written: b-database")
}

// Preferences returns the user's saved preferences; a user who saved none has no overrides.
func (c *ControlService) Preferences(ctx context.Context, user webapi.UserID) (webapi.Preferences, error) {
	panic("not written: b-database")
}

// PatchPreferences merges a patch into the user's preferences with `merge`, which takes
// the saved preferences and answers them patched, and answers them as
// saved. The read, the merge and the write are one transaction, so
// patches of different fields that arrive together all stay.
func (c *ControlService) PatchPreferences(ctx context.Context, user webapi.UserID, merge func(webapi.Preferences) webapi.Preferences) (webapi.Preferences, error) {
	panic("not written: b-database")
}

// OpenWebSession stores a new session and answers when it expires. A login is the one
// moment the table grows, so it also drops the sessions that ran out
// unnoticed.
func (c *ControlService) OpenWebSession(ctx context.Context, token TokenHash, user webapi.UserID, policy SessionPolicy) (core.Timestamp, error) {
	panic("not written: b-database")
}

// ResolveWebSession returns the live session a token hash names, renewed when less than the
// policy's margin remains; an expired session is deleted and is nil.
func (c *ControlService) ResolveWebSession(ctx context.Context, token TokenHash, policy SessionPolicy) (*ResolvedSession, error) {
	panic("not written: b-database")
}

// CloseWebSession deletes the session named by its token hash.
func (c *ControlService) CloseWebSession(ctx context.Context, token TokenHash) error {
	panic("not written: b-database")
}

// IssueEmailChallenge stores a challenge in place of the user's previous one and answers
// when it expires; nil while the previous one was sent less than the
// cooldown ago.
func (c *ControlService) IssueEmailChallenge(ctx context.Context, issue ChallengeIssue, policy ChallengePolicy) (*core.Timestamp, error) {
	panic("not written: b-database")
}

// DeleteEmailChallenge deletes the user's challenge when its ID matches.
func (c *ControlService) DeleteEmailChallenge(ctx context.Context, user webapi.UserID, id string) error {
	panic("not written: b-database")
}

// ConfirmEmailChallenge checks the code and, when it holds, changes the address and consumes
// the challenge, in one transaction with the uniqueness check.
func (c *ControlService) ConfirmEmailChallenge(ctx context.Context, user webapi.UserID, id string, codeHash CodeHash, attempts uint32) (ChallengeOutcome, error) {
	panic("not written: b-database")
}
