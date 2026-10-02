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

// RestoreErrorKind distinguishes opening a tree from continuing it.
type RestoreErrorKind uint8

const (
	// RestoreOpen means the tree did not open.
	RestoreOpen RestoreErrorKind = iota
	// RestoreContinue means the restored tree did not continue.
	RestoreContinue
)

// RestoreError explains why a tree could not be restored without a connection.
type RestoreError struct {
	Kind  RestoreErrorKind
	Cause error
}

// Error returns the restoration failure's text.
func (e *RestoreError) Error() string {
	if e.Kind == RestoreOpen {
		return fmt.Sprintf("the tree did not open: %v", e.Cause)
	}
	return fmt.Sprintf("the restored tree did not continue: %v", e.Cause)
}

// Unwrap preserves the opening or store failure.
func (e *RestoreError) Unwrap() error { return e.Cause }
