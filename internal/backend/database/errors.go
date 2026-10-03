package database

import (
	"errors"
	"fmt"
)

var (
	// ErrClosed means the database is closed.
	ErrClosed = errors.New("the database is closed")
	// ErrCorrupt means a stored value is outside its type; nothing repairs it.
	ErrCorrupt = errors.New("holds an invalid value")
	// ErrTimeRange means a computed time is outside the supported range.
	ErrTimeRange   = errors.New("a time is out of range")
	errSQLite      = errors.New("SQLite failed")
	errOtherSchema = errors.New(
		"was made by another version of Demi; move the data directory away and start with a new one (storage.md § Schemas)",
	)
	errJournalMode = errors.New("the database stays in journal mode")
	errFileSystem  = errors.New("the file system failed")
)

// CorruptValue reports that table.column holds an invalid value, for the
// reason given; errors.Is matches it to ErrCorrupt and to reason.
func CorruptValue(table, column string, reason error) error {
	return fmt.Errorf("%s.%s %w: %w", table, column, ErrCorrupt, reason)
}
