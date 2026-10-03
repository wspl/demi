package database

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
	return ReserveNumbers(ctx, tx, sequence, 1)
}

// ReserveNumbers advances sequence by count and returns the first reserved number.
func ReserveNumbers(ctx context.Context, tx *sql.Tx, sequence core.Sequence, count uint32) (uint64, error) {
	row, err := queryRecord(
		ctx,
		tx,
		"sequences",
		"INSERT INTO sequences (name,next) VALUES (?,1 + ?) ON CONFLICT (name) DO UPDATE "+
			"SET next=next + ? RETURNING next - ? AS next",
		func(r *storedRow) uint64 { return r.count("next") },
		sequence,
		count,
		count,
		count,
	)
	if err != nil {
		return 0, err
	}
	return *row, nil
}

// Sequences returns each used sequence with its next number.
func Sequences(ctx context.Context, tx *sql.Tx) ([]SequenceNext, error) {
	return queryRecords(
		ctx,
		tx,
		"sequences",
		"SELECT name,next FROM sequences ORDER BY name",
		func(r *storedRow) SequenceNext {
			sequence := core.Sequence(r.text("name"))
			r.bad("name", sequence.Validate())
			return SequenceNext{Sequence: sequence, Next: r.count("next")}
		},
	)
}

// ContinueSequences starts a Fork's sequences at the source's next numbers.
func ContinueSequences(ctx context.Context, tx *sql.Tx, sequences []SequenceNext) error {
	for _, sequence := range sequences {
		if err := execSQL(
			ctx,
			tx,
			"INSERT INTO sequences (name,next) VALUES (?,?) ON CONFLICT (name) DO UPDATE SET next=max(next,excluded.next)",
			sequence.Sequence,
			sequence.Next,
		); err != nil {
			return err
		}
	}
	return nil
}
