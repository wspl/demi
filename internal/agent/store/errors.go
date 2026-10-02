package store

import "errors"

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
func (e *Error) Error() string {
	switch e.Kind {
	case Invalidated:
		return "the command storage handle is no longer current"
	case Corrupt:
		return "the stored agent tree is corrupt: " + e.Message
	default:
		if e.Message != "" {
			return e.Message
		}
		if e.Cause != nil {
			return e.Cause.Error()
		}
		return ""
	}
}

// Unwrap returns the underlying operation error.
func (e *Error) Unwrap() error { return e.Cause }

// Is compares store error kinds.
func (e *Error) Is(target error) bool {
	var other *Error
	return errors.As(target, &other) && other != nil && e.Kind == other.Kind
}
