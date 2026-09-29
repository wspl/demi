// Package storage owns the backend's durable records and their validated encodings.
package storage

import (
	"context"
	"database/sql"
	_ "embed"
	"errors"
	"fmt"
	"net/url"
	"path/filepath"
	"strings"
	"time"

	"github.com/wspl/demi/go/core"
	_ "modernc.org/sqlite"
)

// These are the Rust backend's version-one migrations, without changes.
//
//go:embed control_v1.sql
var controlV1 string

//go:embed conversation_v1.sql
var conversationV1 string

var ErrClosed = errors.New("the database is closed")

type CorruptError struct{ Table, Column, Reason string }

func (e *CorruptError) Error() string {
	return fmt.Sprintf("%s.%s holds an invalid value: %s", e.Table, e.Column, e.Reason)
}
func corrupt(table, column string, err error) error {
	if err == nil {
		return nil
	}
	return &CorruptError{table, column, err.Error()}
}
func sqliteError(err error) error {
	if err == nil {
		return nil
	}
	// database/sql keeps its closed-DB sentinel unexported.
	var corrupt *CorruptError
	if errors.As(err, &corrupt) {
		return corrupt
	}
	if errors.Is(err, sql.ErrConnDone) || err.Error() == "sql: database is closed" {
		return ErrClosed
	}
	return fmt.Errorf("SQLite failed: %w", err)
}

type scanner interface{ Scan(...any) error }
type database interface {
	ExecContext(context.Context, string, ...any) (sql.Result, error)
	QueryContext(context.Context, string, ...any) (*sql.Rows, error)
	QueryRowContext(context.Context, string, ...any) *sql.Row
}

// openDatabase gives each writer one serialized connection with the Rust pragmas.
func openDatabase(ctx context.Context, path, schema string) (*sql.DB, error) {
	absolute, err := filepath.Abs(path)
	if err != nil {
		return nil, err
	}
	location := url.URL{Scheme: "file", Path: absolute}
	query := url.Values{"_pragma": {"foreign_keys(1)", "busy_timeout(5000)"}}
	location.RawQuery = query.Encode()
	db, err := sql.Open("sqlite", location.String())
	if err != nil {
		return nil, sqliteError(err)
	}
	db.SetMaxOpenConns(1)
	db.SetMaxIdleConns(1)
	success := false
	defer func() {
		if !success {
			_ = db.Close()
		}
	}()
	fail := func(err error) (*sql.DB, error) { return nil, err }
	var mode string
	if err = db.QueryRowContext(ctx, "PRAGMA journal_mode=WAL").Scan(&mode); err != nil {
		return fail(sqliteError(err))
	}
	if !strings.EqualFold(mode, "wal") {
		return fail(fmt.Errorf("the database stays in journal mode %s, not WAL", mode))
	}
	tx, err := db.BeginTx(ctx, nil)
	if err != nil {
		return fail(sqliteError(err))
	}
	defer tx.Rollback() // A committed transaction is already closed.
	var version int
	if err = tx.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version); err != nil {
		return fail(sqliteError(err))
	}
	if version > 1 {
		return fail(fmt.Errorf("the schema could not be applied: database version %d is newer than version 1", version))
	}
	if version == 0 {
		if _, err = tx.ExecContext(ctx, schema); err != nil {
			return fail(fmt.Errorf("the schema could not be applied: %w", err))
		}
		if _, err = tx.ExecContext(ctx, "PRAGMA user_version=1"); err != nil {
			return fail(sqliteError(err))
		}
	}
	if err = tx.Commit(); err != nil {
		return fail(sqliteError(err))
	}
	success = true
	return db, nil
}

func instant(table, column string, ms int64) (core.Timestamp, error) {
	value, err := core.TimestampFromMillisecond(ms)
	return value, corrupt(table, column, err)
}
func after(now core.Timestamp, duration time.Duration) (core.Timestamp, error) {
	value, err := core.TimestampFromMillisecond(now.Millisecond() + duration.Milliseconds())
	if err != nil {
		return value, fmt.Errorf("a time is out of range: %w", err)
	}
	return value, nil
}

// storedCount names a corrupt unsigned count at the SQLite boundary.
type storedCount struct {
	table, column string
	target        *uint64
}

func (c storedCount) Scan(value any) error {
	n, ok := value.(int64)
	if !ok {
		return &CorruptError{c.table, c.column, "expected an INTEGER"}
	}
	if n < 0 {
		return &CorruptError{c.table, c.column, "out of range integral type conversion attempted"}
	}
	*c.target = uint64(n)
	return nil
}
