package database

import "errors"

var (
	// ErrInvalidCode means the code is wrong, expired, used up, or no longer holds.
	ErrInvalidCode = errors.New("the verification code is wrong, expired or used up")
	// ErrEmailTaken means another account has the address.
	ErrEmailTaken = errors.New("another account has the email address")
	// ErrCoolingDown means the previous challenge was sent less than the cooldown ago.
	ErrCoolingDown = errors.New("the previous challenge was sent within the cooldown")
)
