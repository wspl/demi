package providers

import "fmt"

// FamilyErrorKind identifies why the operation failed.
type FamilyErrorKind uint8

// FamilyError categories.
const (
	FamilyWrongCredential FamilyErrorKind = 0
	FamilyInvalid         FamilyErrorKind = 1
)

// FamilyError reports a typed provider operation failure.
type FamilyError struct {
	Kind    FamilyErrorKind
	Message string
}

// Error returns the failure message.
func (e *FamilyError) Error() string {
	if e.Kind == FamilyWrongCredential {
		return "the entry's credential is not one its family takes"
	}
	return e.Message
}

// AssemblyErrorKind identifies why the operation failed.
type AssemblyErrorKind uint8

// AssemblyError categories.
const (
	AssemblyStorage          AssemblyErrorKind = 0
	AssemblyUnknownFamily    AssemblyErrorKind = 1
	AssemblyFamily           AssemblyErrorKind = 2
	AssemblyNoProcessRuntime AssemblyErrorKind = 3
)

// AssemblyError reports a typed provider operation failure.
type AssemblyError struct {
	Kind   AssemblyErrorKind
	Family string
	Err    error
}

// Error returns the failure message.
func (e *AssemblyError) Error() string {
	switch e.Kind {
	case AssemblyUnknownFamily:
		return "the provider family " + e.Family + " is not available"
	case AssemblyNoProcessRuntime:
		return "the provider family " + e.Family + " cannot run its process"
	}
	return e.Err.Error()
}

// Unwrap returns the underlying failure.
func (e *AssemblyError) Unwrap() error { return e.Err }

// AccountRefusalKind identifies why the operation failed.
type AccountRefusalKind uint8

// AccountRefusal categories.
const (
	AccountExists            AccountRefusalKind = 0
	AccountUnsupported       AccountRefusalKind = 1
	AccountNotFound          AccountRefusalKind = 2
	AccountActive            AccountRefusalKind = 3
	AccountTokenImportFailed AccountRefusalKind = 4
	AccountStore             AccountRefusalKind = 5
	AccountAssembly          AccountRefusalKind = 6
)

// AccountRefusal reports a typed provider operation failure.
type AccountRefusal struct {
	Kind    AccountRefusalKind
	Message string
	Err     error
}

// Error returns the failure message.
func (e *AccountRefusal) Error() string {
	switch e.Kind {
	case AccountExists:
		return "Add this token to the existing Claude Code provider"
	case AccountNotFound:
		return "No such account"
	case AccountActive:
		return "Select another account before removing the active one, or delete the provider"
	case AccountTokenImportFailed:
		return "The setup token could not be imported"
	case AccountAssembly:
		return e.Err.Error()
	}
	return e.Message
}

// Unwrap returns the underlying failure.
func (e *AccountRefusal) Unwrap() error { return e.Err }

// LoginRefusalKind identifies why the operation failed.
type LoginRefusalKind uint8

// LoginRefusal categories.
const (
	LoginNoLoginFlow LoginRefusalKind = 0
	LoginExists      LoginRefusalKind = 1
	LoginBusy        LoginRefusalKind = 2
	LoginAssembly    LoginRefusalKind = 3
)

// LoginRefusal reports a typed provider operation failure.
type LoginRefusal struct {
	Kind   LoginRefusalKind
	Family string
	Err    error
}

// Error returns the failure message.
func (e *LoginRefusal) Error() string {
	switch e.Kind {
	case LoginNoLoginFlow:
		return e.Family + " has no device login"
	case LoginExists:
		return "This scope already has a " + e.Family + " subscription"
	case LoginBusy:
		return "Another provider operation is still running"
	}
	return e.Err.Error()
}

// Unwrap returns the underlying failure.
func (e *LoginRefusal) Unwrap() error { return e.Err }

// RateLimited reports a request that exceeds the limit; it never reaches the vendor and does not count.
type RateLimited struct{ Limit int }

// Error returns the failure message.
func (e *RateLimited) Error() string {
	return fmt.Sprintf("Provider request rate limit reached (%d per minute)", e.Limit)
}

// NotConfigured reports a model the configured list does not name; its request fails.
type NotConfigured struct{ Model string }

// Error returns the failure message.
func (e *NotConfigured) Error() string {
	return "the model " + e.Model + " is not in the provider's configured list"
}

// Unsealable reports a sealed value that was altered, moved, or sealed under another key.
type Unsealable struct{}

// Error returns the failure message.
func (e *Unsealable) Error() string { return "the sealed value does not open" }

// ReleaseError reports why the newest release could not be read.
type ReleaseError struct{ Message string }

// Error returns the failure message.
func (e *ReleaseError) Error() string { return e.Message }
