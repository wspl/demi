//revive:disable:unused-parameter // API checkpoint keeps parameter names for callers; bodies follow after merge.
package providers

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
func (e *FamilyError) Error() string { panic("not written: b-providers") }

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
func (e *AssemblyError) Error() string { panic("not written: b-providers") }

// Unwrap returns the underlying failure.
func (e *AssemblyError) Unwrap() error { panic("not written: b-providers") }

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
func (e *AccountRefusal) Error() string { panic("not written: b-providers") }

// Unwrap returns the underlying failure.
func (e *AccountRefusal) Unwrap() error { panic("not written: b-providers") }

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
func (e *LoginRefusal) Error() string { panic("not written: b-providers") }

// Unwrap returns the underlying failure.
func (e *LoginRefusal) Unwrap() error { panic("not written: b-providers") }

// RateLimited reports A request that exceeds the limit; it never reaches the vendor and does not count.
type RateLimited struct{ Limit int }

// Error returns the failure message.
func (e *RateLimited) Error() string { panic("not written: b-providers") }

// NotConfigured reports A model the configured list does not name; its request fails.
type NotConfigured struct{ Model string }

// Error returns the failure message.
func (e *NotConfigured) Error() string { panic("not written: b-providers") }

// Unsealable reports A sealed value that was altered, moved, or sealed under another key.
type Unsealable struct{}

// Error returns the failure message.
func (e *Unsealable) Error() string { panic("not written: b-providers") }

// ReleaseError reports Why the newest release could not be read.
type ReleaseError struct{ Message string }

// Error returns the failure message.
func (e *ReleaseError) Error() string { panic("not written: b-providers") }
