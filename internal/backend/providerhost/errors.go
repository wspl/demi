package providerhost

import "errors"

// ErrWrongCredential means an entry's credential is not one its family takes.
var ErrWrongCredential = errors.New("the entry's credential is not one its family takes")

// Account and login refusals the edge answers with their own codes.
//
//nolint:staticcheck // Product text, shown to the user as it is.
var (
	// ErrSetupTokenUnavailable means no registered family imports setup tokens.
	ErrSetupTokenUnavailable = errors.New("Setup-token import is unavailable")
	// ErrUseDeviceLogin means the provider adds accounts only through device login.
	ErrUseDeviceLogin = errors.New("Use device login for this provider")
	// ErrNotSubscription means the provider does not use subscription accounts.
	ErrNotSubscription = errors.New("This provider does not use subscription accounts")
	// ErrSetupTokenProviderExists means the user already has a Claude Code provider.
	ErrSetupTokenProviderExists = errors.New("Add this token to the existing Claude Code provider")
	// ErrAccountNotFound means the requested account does not exist.
	ErrAccountNotFound = errors.New("No such account")
	// ErrActiveAccount means the operation would remove the selected account.
	ErrActiveAccount = errors.New("Select another account before removing the active one, or delete the provider")
	// ErrTokenImportFailed means the setup token could not be imported.
	ErrTokenImportFailed = errors.New("The setup token could not be imported")
	// ErrLoginBusy means another operation on the provider is still running.
	ErrLoginBusy = errors.New("Another provider operation is still running")
)

// errUnsealable means a sealed value was altered, moved, or sealed under another key.
var errUnsealable = errors.New("the sealed value does not open")

// AssemblyErrorKind identifies why a provider could not be assembled.
type AssemblyErrorKind uint8

// AssemblyError categories.
const (
	// AssemblyStorage means provider storage could not be read.
	AssemblyStorage AssemblyErrorKind = 0
	// AssemblyUnknownFamily means the entry names an unavailable family.
	AssemblyUnknownFamily AssemblyErrorKind = 1
)

// AssemblyError reports a provider that could not be assembled. The edge tells
// a storage failure from others, and an unknown family in a login from others.
type AssemblyError struct {
	// Kind selects the failure category.
	Kind AssemblyErrorKind
	// Family names the unknown family.
	Family string
	// Err is the storage failure.
	Err error
}

// Error returns the failure message.
func (e *AssemblyError) Error() string {
	if e.Kind == AssemblyUnknownFamily {
		return "the provider family " + e.Family + " is not available"
	}
	return e.Err.Error()
}

// Unwrap returns the storage failure.
func (e *AssemblyError) Unwrap() error {
	return e.Err
}

// LoginErrorKind identifies why a login did not start or complete.
type LoginErrorKind uint8

// LoginError categories.
const (
	// LoginNoLoginFlow means the family has no subscription login flow.
	LoginNoLoginFlow LoginErrorKind = 0
	// LoginExists means the scope already has this subscription family.
	LoginExists LoginErrorKind = 1
	// LoginAssembly means the login's provider could not be assembled.
	LoginAssembly LoginErrorKind = 2
)

// LoginError reports why a login did not start or complete.
type LoginError struct {
	// Kind selects the failure category.
	Kind LoginErrorKind
	// Family names the provider family involved.
	Family string
	// Err is the assembly failure.
	Err error
}

// Error returns the failure message.
func (e *LoginError) Error() string {
	switch e.Kind {
	case LoginNoLoginFlow:
		return e.Family + " has no device login"
	case LoginExists:
		return "This scope already has a " + e.Family + " subscription"
	}
	return e.Err.Error()
}

// Unwrap returns the assembly failure.
func (e *LoginError) Unwrap() error {
	return e.Err
}
