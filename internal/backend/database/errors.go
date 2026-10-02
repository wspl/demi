package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import "errors"

// ErrClosed means the database is closed.
var ErrClosed = errors.New("the database is closed")

// Error describes why a storage operation failed. Use errors.As for details and
// errors.Is on the wrapped cause. Closed operations return ErrClosed.
type Error struct {
	Kind   ErrorKind
	Path   string
	Table  string
	Column string
	Reason string
	Err    error
}

// ErrorKind identifies storage failures with distinct diagnostic context.
type ErrorKind uint8

const (
	// SQLiteFailure wraps a database driver error.
	SQLiteFailure ErrorKind = iota
	// OtherSchema means this build neither reads nor changes the file's schema.
	OtherSchema
	// JournalMode means the filesystem refused WAL.
	JournalMode
	// Corrupt means a stored value is outside its type and is never repaired.
	Corrupt
	// TimeRange means a computed time is outside the supported range.
	TimeRange
	// IOFailure wraps a filesystem failure.
	IOFailure
)

// Error describes the failed operation using the Rust storage error wording.
func (e *Error) Error() string { panic("not written: b-database") }

// Unwrap exposes the underlying cause to errors.Is and errors.As.
func (e *Error) Unwrap() error { panic("not written: b-database") }
