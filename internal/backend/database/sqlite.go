package database

import (
	"context"
	"crypto/sha256"
	"database/sql"
	_ "embed"
	"encoding/binary"
	"errors"
	"fmt"
	"net/url"
	"path/filepath"
	"strings"

	"modernc.org/sqlite"
)

// A schema's version is derived from the SHA-256 digest of its text, so any byte
// changed in a schema file is another schema (storage.md § Schemas).
//
//go:embed control_schema.sql
var controlSchema string

//go:embed conversation_schema.sql
var conversationSchema string

func schemaVersion(schema string) uint32 {
	digest := sha256.Sum256([]byte(schema))
	return max(1, binary.BigEndian.Uint32(digest[:4])>>1)
}

// openSQLite configures a single database writer or a short-lived cold reader.
func openSQLite(ctx context.Context, path, schema string, readonly bool) (_ *sql.DB, err error) {
	absolute, err := filepath.Abs(path)
	if err != nil {
		return nil, fmt.Errorf("%w: %w", errFileSystem, err)
	}
	u := url.URL{Scheme: "file", Path: filepath.ToSlash(absolute)}
	q := url.Values{}
	q.Add("_pragma", "busy_timeout(5000)")
	if readonly {
		q.Set("mode", "ro")
	} else {
		q.Add("_pragma", "journal_mode(WAL)")
		q.Add("_pragma", "foreign_keys(1)")
	}
	u.RawQuery = q.Encode()
	db, err := sql.Open("sqlite", u.String())
	if err != nil {
		return nil, sqlError(err)
	}
	db.SetMaxOpenConns(1)
	db.SetMaxIdleConns(1)
	defer func() {
		if err != nil {
			err = errors.Join(err, db.Close())
		}
	}()
	if readonly {
		return db, nil
	}
	err = transaction(ctx, db, func(ctx context.Context, tx *sql.Tx) error {
		return initializeSchema(ctx, tx, path, schema)
	}, nil)
	if err != nil {
		return nil, err
	}
	return db, nil
}

// transaction owns an admitted SQLite operation through rollback or commit.
// Cancellation applies to admission; once admitted, its commit is not abandoned.
func transaction(
	ctx context.Context,
	db *sql.DB,
	work func(context.Context, *sql.Tx) error,
	commit func(context.Context, *sql.Tx) error,
) (err error) {
	conn, err := db.Conn(ctx)
	if err != nil {
		return sqlError(err)
	}
	defer func() { err = errors.Join(err, conn.Close()) }()
	operation := context.WithoutCancel(ctx)
	tx, err := conn.BeginTx(operation, nil)
	if err != nil {
		return sqlError(err)
	}
	defer func() {
		rollback := tx.Rollback()
		if rollback != nil && !errors.Is(rollback, sql.ErrTxDone) {
			err = errors.Join(err, sqlError(rollback))
		}
		if recovered := recover(); recovered != nil {
			err = fmt.Errorf("storage operation panicked: %v", recovered)
		}
	}()
	if err = work(operation, tx); err != nil {
		return sqlError(err)
	}
	if commit != nil {
		return sqlError(commit(operation, tx))
	}
	return sqlError(tx.Commit())
}

func sqlError(err error) error {
	if err == nil {
		return nil
	}
	if errors.Is(err, ErrCorrupt) || errors.Is(err, ErrTimeRange) || errors.Is(err, errSQLite) ||
		errors.Is(err, errOtherSchema) ||
		errors.Is(err, errJournalMode) ||
		errors.Is(err, errFileSystem) ||
		errors.Is(err, ErrClosed) ||
		errors.Is(err, context.Canceled) ||
		errors.Is(err, context.DeadlineExceeded) {
		return err
	}
	var sqliteErr *sqlite.Error
	if errors.As(err, &sqliteErr) {
		return fmt.Errorf("%w: %w", errSQLite, err)
	}
	// Domain refusals must remain distinguishable from a SQLite failure.
	return fmt.Errorf("storage operation: %w", err)
}

func initializeSchema(ctx context.Context, tx *sql.Tx, path, schema string) error {
	var mode string
	if err := tx.QueryRowContext(ctx, "PRAGMA journal_mode").Scan(&mode); err != nil {
		return err
	}
	if !strings.EqualFold(mode, "wal") {
		return fmt.Errorf("%w %s, not WAL", errJournalMode, mode)
	}
	var version uint32
	if err := tx.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version); err != nil {
		return err
	}
	if version == schemaVersion(schema) {
		return nil
	}
	var tables int
	if err := tx.QueryRowContext(ctx, "SELECT count(*) FROM sqlite_schema WHERE type = 'table'").
		Scan(&tables); err != nil {
		return err
	}
	if version != 0 || tables != 0 {
		return fmt.Errorf("%s %w", path, errOtherSchema)
	}
	if _, err := tx.ExecContext(ctx, schema); err != nil {
		return err
	}
	_, err := tx.ExecContext(ctx, fmt.Sprintf("PRAGMA user_version = %d", schemaVersion(schema)))
	return err
}
