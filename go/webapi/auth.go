package webapi

import (
	"github.com/wspl/demi/go/core"
)

// An account's role (`product.md` § User system). A role administers only
// the roles below it.
//
//demi:enum
//demi:export
type Role string

const (
	RoleAdmin Role = "admin"
	RoleUser  Role = "user"
	// The instance's first account, created by setup.
	RoleMaster Role = "master"
)

// An account as the browser sees it; its password hash never leaves the
// backend.
//
//demi:wire open
type UserDTO struct {
	ID    UserID       `json:"id" check:"func=Validate"`
	Email EmailAddress `json:"email" check:"func=Validate"`
	// Empty until the user chooses one.
	Nickname  string         `json:"nickname"`
	Role      Role           `json:"role"`
	CreatedAt core.Timestamp `json:"createdAt" check:"func=core.Validate"`
}

// `{ user }`: the answer of setup, login, `GET/PATCH /auth/me` and a
// confirmed email change.
//
//demi:wire open
type Identity struct {
	User UserDTO `json:"user"`
}

// `GET /setup`: whether the instance still needs its master account.
//
//demi:wire open
type SetupStatus struct {
	Needed bool `json:"needed"`
}

// `POST /setup`: the master account's address and password.
//
//demi:wire
type SetupRequest struct {
	// An `EmailAddress` is valid once it is decoded.
	Email    EmailAddress `json:"email" check:"func=Validate"`
	Password Password     `json:"password" check:"chars=8..1024,func=Validate"`
}

// `POST /auth/login`.
//
//demi:wire
type Credentials struct {
	// An `EmailAddress` is valid once it is decoded.
	Email    EmailAddress `json:"email" check:"func=Validate"`
	Password Password     `json:"password" check:"chars=1..,func=Validate"`
}

// `PATCH /auth/me`.
//
//demi:wire
type NicknamePatch struct {
	Nickname Trimmed `json:"nickname" check:"chars=1..80,func=Validate"`
}

// `PUT /auth/password`: the current password and the next one.
//
//demi:wire
type PasswordChange struct {
	Current Password `json:"current" check:"chars=1..,func=Validate"`
	Next    Password `json:"next" check:"chars=8..1024,func=Validate"`
}

// `POST /auth/email`: the new address and the current password.
//
//demi:wire
type EmailChangeStart struct {
	// An `EmailAddress` is valid once it is decoded.
	Email    EmailAddress `json:"email" check:"func=Validate"`
	Password Password     `json:"password" check:"chars=1..,func=Validate"`
}

// A pending email change; its code went to `email` and never appears here.
//
//demi:wire open
type EmailChallengeDTO struct {
	ID        string         `json:"id"`
	Email     EmailAddress   `json:"email" check:"func=Validate"`
	ExpiresAt core.Timestamp `json:"expiresAt" check:"func=core.Validate"`
}

// The 202 answer of `POST /auth/email`.
//
//demi:wire open
type EmailChangeStarted struct {
	Challenge EmailChallengeDTO `json:"challenge"`
}

// `POST /auth/email/confirm`: the challenge and the six-digit code the new
// address received.
//
//demi:wire
type EmailChangeConfirm struct {
	ID   string `json:"id" check:"bytes=1.."`
	Code string `json:"code" check:"pattern=EmailChangeConfirmCodePattern"`
}
