package database

import (
	"time"

	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// Account is an account with the hash its password checks against.
type Account struct {
	User         webapiproto.UserDTO
	PasswordHash PasswordHash
}

// SessionPolicy describes how long a session lives after its last renewal, and how little of that
// must remain for a request to renew it.
type SessionPolicy struct {
	Lifetime   time.Duration
	RenewBelow time.Duration
}

// ResolvedSession is a live session's user and expiry, and whether this request renewed it.
type ResolvedSession struct {
	User      webapiproto.UserDTO
	ExpiresAt types.Timestamp
	Renewed   bool
}

// ChallengePolicy describes how long a code lasts, how soon another may be sent, and how many wrong
// codes a challenge takes.
type ChallengePolicy struct {
	Lifetime time.Duration
	Cooldown time.Duration
	Attempts uint32
}

// ChallengeIssue is a challenge to store: the new address, and the password hash it was
// issued under, so a password change ends it.
type ChallengeIssue struct {
	User         webapiproto.UserID
	ID           string
	Email        webapiproto.EmailAddress
	PasswordHash PasswordHash
	CodeHash     CodeHash
}
