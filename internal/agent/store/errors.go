package store

// API checkpoint: named parameters document the interface until bodies are ported.
//revive:disable:unused-parameter

// ErrorKind identifies why a store operation failed.
type ErrorKind uint8

const (
	// Invalidated means a history rewrite or dispose made the save stale.
	Invalidated ErrorKind = iota
	// Corrupt means stored data is not a valid record or checkpoint.
	Corrupt
	// OperationFailed means the operation failed, such as a refused transaction.
	OperationFailed
)

// Error describes a store failure and preserves its underlying cause.
// Errors compare by Kind through errors.Is; errors.As exposes the details.
type Error struct {
	Kind    ErrorKind
	Message string
	Cause   error
}

// ErrInvalidated means the command storage handle is no longer current.
var ErrInvalidated = &Error{Kind: Invalidated}

// Error renders the Rust store error's user-facing text.
func (e *Error) Error() string { panic("not written: a-store") }

// Unwrap returns the underlying operation error.
func (e *Error) Unwrap() error { panic("not written: a-store") }

// Is compares store error kinds.
func (e *Error) Is(target error) bool { panic("not written: a-store") }
