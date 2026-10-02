package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"
	"database/sql"

	"github.com/wspl/demi/internal/core"
)

// SequenceNext is a sequence and the next number it will issue.
type SequenceNext struct {
	Sequence core.Sequence
	Next     uint64
}

// NextNumber advances sequence and returns its next number, beginning at one.
func NextNumber(ctx context.Context, tx *sql.Tx, sequence core.Sequence) (uint64, error) {
	panic("not written: b-database")
}

// ReserveNumbers advances sequence by count and returns the first reserved number.
func ReserveNumbers(ctx context.Context, tx *sql.Tx, sequence core.Sequence, count uint32) (uint64, error) {
	panic("not written: b-database")
}

// Sequences returns each used sequence with its next number.
func Sequences(ctx context.Context, tx *sql.Tx) ([]SequenceNext, error) {
	panic("not written: b-database")
}

// ContinueSequences starts a Fork's sequences at the source's next numbers.
func ContinueSequences(ctx context.Context, tx *sql.Tx, sequences []SequenceNext) error {
	panic("not written: b-database")
}
