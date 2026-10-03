package server

import (
	"errors"
	"fmt"
)

// ErrWorking means the conversation's tree works, which a reload does not interrupt.
var ErrWorking = errors.New("the conversation's agents are working")

// ResolveErrorKind distinguishes an unavailable provider from failed assembly.
type ResolveErrorKind uint8

const (
	// ResolveUnknown means no provider entry has this id.
	ResolveUnknown ResolveErrorKind = iota
	// ResolveFailed means the entry exists but could not build a runtime.
	ResolveFailed
)

// ResolveError explains why no runtime could be built. Inspect it with errors.As.
type ResolveError struct {
	Kind     ResolveErrorKind
	Provider string
	Message  string
	Cause    error
}

// Error returns the provider-resolution failure's text.
func (e *ResolveError) Error() string {
	if e.Kind == ResolveUnknown {
		return fmt.Sprintf("Provider %q is not available", e.Provider)
	}
	return e.Message
}

// Unwrap preserves the underlying failure for errors.Is and errors.As.
func (e *ResolveError) Unwrap() error { return e.Cause }

//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
var (
	// errNoSession refuses a steer on a connection with no open session.
	errNoSession = errors.New("No session is open on this connection")
	// errQueuedMessageNotFound refuses a steer of a message that is not queued.
	errQueuedMessageNotFound = errors.New("Queued message not found")
)
