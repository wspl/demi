package server

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import "errors"

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
func (e *ResolveError) Error() string { panic("not written: a-server") }

// Unwrap preserves the underlying failure for errors.Is and errors.As.
func (e *ResolveError) Unwrap() error { panic("not written: a-server") }

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
func (e *RestoreError) Error() string { panic("not written: a-server") }

// Unwrap preserves the opening or store failure.
func (e *RestoreError) Unwrap() error { panic("not written: a-server") }
