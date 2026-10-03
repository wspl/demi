package providers

import "fmt"

// FamilyErrorKind identifies why the operation failed.
type FamilyErrorKind uint8

// FamilyError categories.
const (
	// FamilyWrongCredential means the entry’s credential does not match its family.
	FamilyWrongCredential FamilyErrorKind = 0
	// FamilyInvalid means the family refused its configuration.
	FamilyInvalid FamilyErrorKind = 1
)

// FamilyError reports a typed provider operation failure.
type FamilyError struct {
	// Kind selects the failure category and applicable details.
	Kind FamilyErrorKind
	// Message holds the diagnostic for categories with custom text.
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
	// AssemblyStorage means provider storage could not be read.
	AssemblyStorage AssemblyErrorKind = 0
	// AssemblyUnknownFamily means the entry names an unavailable family.
	AssemblyUnknownFamily AssemblyErrorKind = 1
	// AssemblyFamily means the family could not build the provider.
	AssemblyFamily AssemblyErrorKind = 2
	// AssemblyNoProcessRuntime means the family cannot run its process.
	AssemblyNoProcessRuntime AssemblyErrorKind = 3
)

// AssemblyError reports a typed provider operation failure.
type AssemblyError struct {
	// Kind selects the failure category and applicable details.
	Kind AssemblyErrorKind
	// Family identifies the provider family involved in the failure.
	Family string
	// Err retains the underlying failure for errors.Is and errors.As.
	Err error
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
	// AccountExists means this scope already has the token’s provider family.
	AccountExists AccountRefusalKind = 0
	// AccountUnsupported means the provider does not support the account operation.
	AccountUnsupported AccountRefusalKind = 1
	// AccountNotFound means the requested account does not exist.
	AccountNotFound AccountRefusalKind = 2
	// AccountActive means the operation would remove the selected account.
	AccountActive AccountRefusalKind = 3
	// AccountTokenImportFailed means the setup token could not be imported.
	AccountTokenImportFailed AccountRefusalKind = 4
	// AccountStore means account storage refused the operation.
	AccountStore AccountRefusalKind = 5
	// AccountAssembly means the account’s provider could not be assembled.
	AccountAssembly AccountRefusalKind = 6
)

// AccountRefusal reports a typed provider operation failure.
type AccountRefusal struct {
	// Kind selects the failure category and applicable details.
	Kind AccountRefusalKind
	// Message holds the diagnostic for categories with custom text.
	Message string
	// Err retains the underlying failure for errors.Is and errors.As.
	Err error
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
	// LoginNoLoginFlow means the family has no subscription login flow.
	LoginNoLoginFlow LoginRefusalKind = 0
	// LoginExists means the scope already has this subscription family.
	LoginExists LoginRefusalKind = 1
	// LoginBusy means another provider operation holds admission.
	LoginBusy LoginRefusalKind = 2
	// LoginAssembly means the login’s provider could not be assembled.
	LoginAssembly LoginRefusalKind = 3
)

// LoginRefusal reports a typed provider operation failure.
type LoginRefusal struct {
	// Kind selects the failure category and applicable details.
	Kind LoginRefusalKind
	// Family identifies the provider family involved in the failure.
	Family string
	// Err retains the underlying failure for errors.Is and errors.As.
	Err error
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
type RateLimited struct {
	// Limit is the maximum number of requests allowed per minute.
	Limit int
}

// Error returns the failure message.
func (e *RateLimited) Error() string {
	return fmt.Sprintf("Provider request rate limit reached (%d per minute)", e.Limit)
}

// NotConfigured reports a model the configured list does not name; its request fails.
type NotConfigured struct {
	// Model identifies the model absent from the configured catalog.
	Model string
}

// Error returns the failure message.
func (e *NotConfigured) Error() string {
	return "the model " + e.Model + " is not in the provider's configured list"
}

// Unsealable reports a sealed value that was altered, moved, or sealed under another key.
type Unsealable struct{}

// Error returns the failure message.
func (e *Unsealable) Error() string { return "the sealed value does not open" }

// ReleaseError reports why the newest release could not be read.
type ReleaseError struct {
	// Message holds the diagnostic for categories with custom text.
	Message string
}

// Error returns the failure message.
func (e *ReleaseError) Error() string { return e.Message }
