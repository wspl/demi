package provider

import (
	"errors"
	"fmt"

	"github.com/wspl/demi/go/core"
)

type AuthReason string

const (
	AuthReasonMissing AuthReason = "missing"
	AuthReasonInvalid AuthReason = "invalid"
	AuthReasonStore   AuthReason = "store"
	AuthReasonRefresh AuthReason = "refresh"
)

type AuthFailure struct {
	Family string
	Reason AuthReason
	Detail string
}

func (f *AuthFailure) Error() string {
	switch f.Reason {
	case AuthReasonMissing:
		return fmt.Sprintf("No %s account is signed in", f.Family)
	case AuthReasonInvalid:
		return fmt.Sprintf("The %s account cannot be read: %s", f.Family, f.Detail)
	case AuthReasonStore:
		return fmt.Sprintf("The %s account could not be loaded: %s", f.Family, f.Detail)
	default:
		return f.Detail
	}
}
func AccountFailure(family string, err error) *AuthFailure {
	var failure *AuthFailure
	if errors.As(err, &failure) {
		return failure
	}
	if errors.Is(err, ErrMissingDocument) {
		return &AuthFailure{Family: family, Reason: AuthReasonMissing}
	}
	var invalid *SecretDecodeError
	if errors.As(err, &invalid) {
		return &AuthFailure{Family: family, Reason: AuthReasonInvalid, Detail: "its secret document is " + invalid.Error()}
	}
	return &AuthFailure{Family: family, Reason: AuthReasonStore, Detail: err.Error()}
}
func (f *AuthFailure) Failure() ProviderFailure {
	var code ErrorCode
	switch f.Reason {
	case AuthReasonMissing:
		code = AuthMissing
	case AuthReasonInvalid:
		code = AuthInvalid
	case AuthReasonRefresh:
		code = AuthRefreshFailed
	}
	return ProviderFailure{Message: f.Error(), Code: code}
}
func (f *AuthFailure) State() core.AuthState {
	if f.Reason == AuthReasonMissing {
		return core.AuthStateUnauthenticated{Message: new(f.Error())}
	}
	return core.AuthStateError{Message: f.Error()}
}
func (f *AuthFailure) CatalogError() error {
	kind := CatalogUnauthenticated
	if f.Reason == AuthReasonStore {
		kind = CatalogUnavailable
	}
	return &CatalogError{Kind: kind, Message: f.Error()}
}

type QuotaErrorKind string

const (
	QuotaUnauthenticated QuotaErrorKind = "unauthenticated"
	QuotaUnavailable     QuotaErrorKind = "unavailable"
	QuotaInvalid         QuotaErrorKind = "invalid"
)

type QuotaError struct {
	Kind    QuotaErrorKind
	Message string
}

func (e *QuotaError) Error() string { return e.Message }
func (f *AuthFailure) QuotaError() error {
	kind := QuotaUnauthenticated
	if f.Reason == AuthReasonStore {
		kind = QuotaUnavailable
	}
	return &QuotaError{Kind: kind, Message: f.Error()}
}
