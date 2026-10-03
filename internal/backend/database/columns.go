package database

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"math"
	"time"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
)

// storedRow checks SQL column types before a record reaches its caller.
type storedRow struct {
	table  string
	values map[string]any
	err    error
}

func (r *storedRow) bad(column string, err error) {
	if r.err == nil && err != nil {
		r.err = CorruptValue(r.table, column, err)
	}
}

func (r *storedRow) text(column string) string {
	value, ok := r.values[column].(string)
	if !ok {
		r.bad(column, fmt.Errorf("expected text"))
	}
	return value
}

func (r *storedRow) integer(column string) int64 {
	value, ok := r.values[column].(int64)
	if !ok {
		r.bad(column, fmt.Errorf("expected integer"))
	}
	return value
}

func (r *storedRow) count(column string) uint64 {
	v := r.integer(column)
	if v < 0 {
		r.bad(column, fmt.Errorf("negative count"))
		return 0
	}
	return uint64(v)
}
func (r *storedRow) boolean(column string) bool { return r.integer(column) != 0 }
func (r *storedRow) bytes(column string) []byte {
	v, ok := r.values[column].([]byte)
	if !ok {
		r.bad(column, fmt.Errorf("expected blob"))
	}
	return v
}

func (r *storedRow) instant(column string) core.Timestamp {
	t, err := core.TimestampFromMillisecond(r.integer(column))
	r.bad(column, err)
	return t
}

func (r *storedRow) optionalText(column string) *string {
	if r.values[column] == nil {
		return nil
	}
	v := r.text(column)
	return &v
}

func (r *storedRow) optionalInstant(column string) *core.Timestamp {
	if r.values[column] == nil {
		return nil
	}
	v := r.instant(column)
	return &v
}

func checked[T ~string](r *storedRow, column string, parse func(string) (T, error)) T {
	v, err := parse(r.text(column))
	r.bad(column, err)
	return v
}

func optionalChecked[T ~string](r *storedRow, column string, parse func(string) (T, error)) *T {
	if r.values[column] == nil {
		return nil
	}
	v := checked(r, column, parse)
	return &v
}

func storedJSON[T any](r *storedRow, column string, decode func([]byte) (T, error)) T {
	v, err := decode([]byte(r.text(column)))
	r.bad(column, err)
	return v
}

func optionalJSON[T any](r *storedRow, column string, decode func([]byte) (T, error)) *T {
	if r.values[column] == nil {
		return nil
	}
	v := storedJSON(r, column, decode)
	return &v
}

// queryRecords decodes and validates every selected SQL row and owns its result set.
func queryRecords[T any](
	ctx context.Context,
	tx *sql.Tx,
	table, query string,
	decode func(*storedRow) T,
	args ...any,
) (result []T, err error) {
	rows, err := tx.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer func() { err = errors.Join(err, rows.Close()) }()
	columns, err := rows.Columns()
	if err != nil {
		return nil, err
	}
	result = []T{}
	for rows.Next() {
		values := make([]any, len(columns))
		dest := make([]any, len(columns))
		for i := range values {
			dest[i] = &values[i]
		}
		if err = rows.Scan(dest...); err != nil {
			return nil, err
		}
		row := &storedRow{table: table, values: make(map[string]any, len(columns))}
		for i, name := range columns {
			row.values[name] = values[i]
		}
		value := decode(row)
		if row.err != nil {
			return nil, row.err
		}
		result = append(result, value)
	}
	return result, rows.Err()
}

func queryRecord[T any](
	ctx context.Context,
	tx *sql.Tx,
	table, query string,
	decode func(*storedRow) T,
	args ...any,
) (T, bool, error) {
	var zero T
	rows, err := queryRecords(ctx, tx, table, query, decode, args...)
	if err != nil || len(rows) == 0 {
		return zero, false, err
	}
	return rows[0], true, nil
}

func execSQL(ctx context.Context, tx *sql.Tx, query string, args ...any) error {
	_, err := tx.ExecContext(ctx, query, args...)
	return err
}

func encoded(value any) (string, error) {
	data, err := contract.EncodeJSON(value)
	return string(data), err
}

func later(now core.Timestamp, by time.Duration) (core.Timestamp, error) {
	t, err := now.Time()
	if err != nil {
		return "", fmt.Errorf("%w: %w", ErrTimeRange, err)
	}
	result, err := core.TimestampFromTime(t.Add(by))
	if err != nil {
		return "", fmt.Errorf("%w: %w", ErrTimeRange, err)
	}
	return result, nil
}
func integer(value uint64) int64 { return int64(min(value, math.MaxInt64)) }
