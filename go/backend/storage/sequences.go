package storage

import (
	"context"
	"database/sql"

	"github.com/wspl/demi/go/core"
)

type SequenceNext struct {
	Sequence core.Sequence
	Next     uint64
}

func (d *ConversationDB) Next(ctx context.Context, sequence core.Sequence) (uint64, error) {
	return d.Reserve(ctx, sequence, 1)
}
func (d *ConversationDB) Reserve(ctx context.Context, sequence core.Sequence, count uint32) (uint64, error) {
	db, release, err := d.writer(ctx)
	if err != nil {
		return 0, err
	}
	defer release()
	var number uint64
	err = db.QueryRowContext(ctx, "INSERT INTO sequences(name,next) VALUES (?1,1+?2) ON CONFLICT(name) DO UPDATE SET next=next+?2 RETURNING next-?2", string(sequence), count).Scan(&number)
	return number, sqliteError(err)
}
func (d *ConversationDB) Sequences(ctx context.Context) ([]SequenceNext, error) {
	values := []SequenceNext{}
	_, err := d.read(ctx, func(tx *sql.Tx) error {
		rows, err := tx.QueryContext(ctx, "SELECT name,next FROM sequences ORDER BY name")
		if err != nil {
			return sqliteError(err)
		}
		defer rows.Close()
		for rows.Next() {
			var v SequenceNext
			if err = rows.Scan(&v.Sequence, storedCount{"sequences", "next", &v.Next}); err != nil {
				return sqliteError(err)
			}
			if err = core.ValidateSequence(v.Sequence); err != nil {
				return corrupt("sequences", "name", err)
			}
			values = append(values, v)
		}
		return sqliteError(rows.Err())
	})
	return values, err
}
func continueSequences(ctx context.Context, db database, values []SequenceNext) error {
	for _, v := range values {
		if _, err := db.ExecContext(ctx, "INSERT INTO sequences(name,next) VALUES (?,?) ON CONFLICT(name) DO UPDATE SET next=MAX(next,excluded.next)", string(v.Sequence), v.Next); err != nil {
			return sqliteError(err)
		}
	}
	return nil
}
func (d *ConversationDB) ContinueSequences(ctx context.Context, values []SequenceNext) error {
	return d.transaction(ctx, func(tx *sql.Tx) error { return continueSequences(ctx, tx, values) })
}
