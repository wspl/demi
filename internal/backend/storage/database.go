// Package storage is the backend's storage module: the SQLite layer, the
// numbered control and conversation migrations, the control service over
// control.sqlite, the per-conversation databases, the user blob namespaces
// and the change objects.
//
// A backend data directory holds
//
//	control.sqlite               deployment-wide product records
//	conversations/<id>.sqlite    one agent tree per conversation
//	blobs/<userId>/<sha256>      user-owned attachment and media bytes
//	changes/<conversationId>/    a conversation's edited-file contents
//
// with the same schemas and file layout as the TypeScript backend, so a data
// directory is the same thing for both. The design is docs/demi-next/storage.md.
package storage

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"

	// The pure Go SQLite driver registers itself as "sqlite".
	_ "modernc.org/sqlite"
)

// Querier runs statements. Both *sql.DB and *sql.Tx implement it; code that
// must be atomic with its caller takes the caller's Querier.
type Querier interface {
	ExecContext(ctx context.Context, query string, args ...any) (sql.Result, error)
	QueryContext(ctx context.Context, query string, args ...any) (*sql.Rows, error)
	QueryRowContext(ctx context.Context, query string, args ...any) *sql.Row
}

// Database is the thin storage seam: hand-written SQL through a Querier,
// either statement by statement or inside one transaction.
//
// fn must use only the Querier it is given. A database has one connection, so
// a statement issued on the Database itself from inside fn would wait for fn
// to finish.
type Database interface {
	// Use runs fn outside a transaction: each statement commits on its own.
	Use(ctx context.Context, fn func(q Querier) error) error
	// Transaction runs fn atomically. It commits when fn returns nil and rolls
	// back when fn returns an error, returning that error.
	Transaction(ctx context.Context, fn func(q Querier) error) error
}

// SQLiteDB is one SQLite database file in WAL mode with foreign keys on.
type SQLiteDB struct {
	db *sql.DB
}

var _ Database = (*SQLiteDB)(nil)

// OpenSQLite opens the database at path, creating the file and its directory
// when missing. The path ":memory:" opens a private in-memory database.
func OpenSQLite(path string) (*SQLiteDB, error) {
	if path != ":memory:" {
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			return nil, fmt.Errorf("create database directory: %w", err)
		}
	}
	query := url.Values{}
	// busy_timeout covers the moment a conversation database evicted from
	// the connection cache is still finishing while it reopens.
	query.Add("_pragma", "busy_timeout(5000)")
	query.Add("_pragma", "journal_mode(WAL)")
	query.Add("_pragma", "foreign_keys(1)")
	db, err := sql.Open("sqlite", "file:"+path+"?"+query.Encode())
	if err != nil {
		return nil, fmt.Errorf("open %s: %w", path, err)
	}
	// One connection: statements and transactions are serialized, as with the
	// single handle the design describes, and an in-memory database lives as
	// long as the SQLiteDB.
	db.SetMaxOpenConns(1)
	db.SetMaxIdleConns(1)
	db.SetConnMaxLifetime(0)
	db.SetConnMaxIdleTime(0)
	if err := db.Ping(); err != nil {
		closeErr := db.Close()
		return nil, errors.Join(fmt.Errorf("open %s: %w", path, err), closeErr)
	}
	return &SQLiteDB{db: db}, nil
}

// Use implements Database.
func (d *SQLiteDB) Use(ctx context.Context, fn func(q Querier) error) error {
	return fn(d.db)
}

// Transaction implements Database.
func (d *SQLiteDB) Transaction(ctx context.Context, fn func(q Querier) error) error {
	tx, err := d.db.BeginTx(ctx, nil)
	if err != nil {
		return fmt.Errorf("begin transaction: %w", err)
	}
	if err := fn(tx); err != nil {
		if rollbackErr := tx.Rollback(); rollbackErr != nil {
			return errors.Join(err, fmt.Errorf("roll back: %w", rollbackErr))
		}
		return err
	}
	if err := tx.Commit(); err != nil {
		return fmt.Errorf("commit: %w", err)
	}
	return nil
}

// Close closes the database. Statements already running finish first.
func (d *SQLiteDB) Close() error {
	return d.db.Close()
}
