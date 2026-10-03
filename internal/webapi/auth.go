package webapi

import (
	"fmt"

	"github.com/wspl/demi/internal/core"
)

// An account's role (`product.md` § User system). A role administers only
// the roles below it.
// +demi:enum master admin user
type Role string

// Values of the preceding enumeration.
const (
	// The instance's first account, created by setup.
	RoleMaster Role = "master"
	RoleAdmin  Role = "admin"
	RoleUser   Role = "user"
)

// An account as the web app sees it; its password hash never leaves the
// backend.
// +demi:tolerant
type UserDTO struct {
	ID    UserID       `json:"id"`
	Email EmailAddress `json:"email"`
	// Empty until the user chooses one.
	Nickname  string         `json:"nickname"`
	Role      Role           `json:"role"`
	CreatedAt core.Timestamp `json:"createdAt"`
}

// `{ user }`: the answer of setup, login, `GET/PATCH /auth/me` and a
// confirmed email change.
// +demi:root direction=receive output=web
// +demi:tolerant
type Identity struct {
	User UserDTO `json:"user"`
}

// `GET /setup`: whether the instance still needs its master account.
// +demi:root direction=receive output=web
// +demi:tolerant
type SetupStatus struct {
	Needed bool `json:"needed"`
}

// `POST /setup`: the master account's address and password.
// +demi:root direction=send output=web
type SetupRequest struct {
	// An `EmailAddress` is valid once it is decoded.
	Email EmailAddress `json:"email"`
	// +demi:length chars min=8 max=1024
	Password Password `json:"password"`
}

// `POST /auth/login`.
// +demi:root direction=send output=web
type Credentials struct {
	// An `EmailAddress` is valid once it is decoded.
	Email EmailAddress `json:"email"`
	// +demi:length chars min=1
	Password Password `json:"password"`
}

// `PATCH /auth/me`.
// +demi:root direction=send output=web
type NicknamePatch struct {
	// +demi:length chars min=1 max=80
	Nickname Trimmed `json:"nickname"`
}

// `PUT /auth/password`: the current password and the next one.
// +demi:root direction=send output=web
type PasswordChange struct {
	// +demi:length chars min=1
	Current Password `json:"current"`
	// +demi:length chars min=8 max=1024
	Next Password `json:"next"`
}

// `POST /auth/email`: the new address and the current password.
// +demi:root direction=send output=web
type EmailChangeStart struct {
	// An `EmailAddress` is valid once it is decoded.
	Email EmailAddress `json:"email"`
	// +demi:length chars min=1
	Password Password `json:"password"`
}

// A pending email change; its code went to `email` and never appears here.
// +demi:tolerant
type EmailChallengeDTO struct {
	ID        string         `json:"id"`
	Email     EmailAddress   `json:"email"`
	ExpiresAt core.Timestamp `json:"expiresAt"`
}

// The 202 answer of `POST /auth/email`.
// +demi:root direction=receive output=web
// +demi:tolerant
type EmailChangeStarted struct {
	Challenge EmailChallengeDTO `json:"challenge"`
}

// `POST /auth/email/confirm`: the challenge and the six-digit code the new
// address received.
// +demi:root direction=send output=web
type EmailChangeConfirm struct {
	// +demi:length chars min=1
	ID string `json:"id"`
	// +demi:pattern ^[0-9]{6}$
	Code string `json:"code"`
}

// A password as a request carries it. `Debug` never shows it.
// +demi:schema-primitive
type Password string

// Outranks reports whether this role administers other: a role acts only on the roles
// below it, so nobody acts on a peer or on the master.
func (r Role) Outranks(other Role) bool {
	rank := map[Role]int{RoleMaster: 2, RoleAdmin: 1, RoleUser: 0}
	ownRank, ok := rank[r]
	otherRank, otherOK := rank[other]
	return ok && otherOK && ownRank > otherRank
}

// Format keeps a request's password out of diagnostic output.
func (Password) Format(state fmt.State, _ rune) {
	// fmt.State writes into the caller's formatter. Its error cannot be
	// returned by fmt.Formatter; fmt owns the destination and reports IO errors.
	_, _ = state.Write([]byte("Password(..)"))
}
