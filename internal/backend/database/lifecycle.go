package database

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

// ControlService owns the deployment's control records and single writer.
// Each operation owns one transaction. Close drains admitted operations.
type ControlService struct {
	db      *sql.DB
	clock   core.Clock
	mu      sync.Mutex
	closed  bool
	active  int
	changed chan struct{}
}

// OpenControl opens the database, and gives a new one its schema.
func OpenControl(ctx context.Context, path string, clock core.Clock) (*ControlService, error) {
	db, err := openSQLite(ctx, path, controlSchema, false)
	if err != nil {
		return nil, err
	}
	return &ControlService{db: db, clock: clock, changed: make(chan struct{})}, nil
}

// controlCall admits a control operation and captures its timestamp inside its transaction.
func controlCall[T any](
	ctx context.Context,
	c *ControlService,
	work func(context.Context, *sql.Tx, core.Timestamp) (T, error),
) (result T, err error) {
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return result, ErrClosed
	}
	c.active++
	c.mu.Unlock()
	defer func() {
		c.mu.Lock()
		c.active--
		close(c.changed)
		c.changed = make(chan struct{})
		c.mu.Unlock()
	}()
	err = transaction(ctx, c.db, func(ctx context.Context, tx *sql.Tx) error {
		var e error
		result, e = work(ctx, tx, c.clock.Now())
		return e
	}, nil)
	return
}

// MaxWriters is the most writer connections open at once.
const MaxWriters = 64

// ConversationStores owns lazy conversation handles and the bounded writer LRU.
// Cold reads use independently bounded read-only connections and create no files.
type ConversationStores struct {
	directory string
	limit     int
	mu        sync.Mutex
	writers   map[string]*writer
	tick      uint64
	closed    bool
	active    int
	changed   chan struct{}
	hold      *CommitHold
}
type writer struct {
	db    *sql.DB
	ready chan struct{}
	pins  int
	used  uint64
}

// ConversationDB is a stable lazy conversation handle.
type ConversationDB struct {
	stores *ConversationStores
	file   string
}

// OpenConversations opens the directory with at most maxWriters writers, in 1..64.
func OpenConversations(ctx context.Context, directory string, maxWriters int) (*ConversationStores, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if maxWriters < 1 || maxWriters > MaxWriters {
		return nil, fmt.Errorf("writer count must be between 1 and %d", MaxWriters)
	}
	if err := os.MkdirAll(directory, 0o755); err != nil {
		return nil, &Error{Kind: IOFailure, Err: err}
	}
	return &ConversationStores{
		directory: directory,
		limit:     maxWriters,
		writers:   make(map[string]*writer),
		changed:   make(chan struct{}),
	}, nil
}

// DB returns a stable lazy handle, comparing conversation IDs without case.
func (s *ConversationStores) DB(conversation webapi.ConversationID) *ConversationDB {
	return &ConversationDB{stores: s, file: strings.ToLower(string(conversation))}
}

// Read runs work in a cold read transaction; false means no database exists yet.
func (s *ConversationStores) Read(
	ctx context.Context,
	conversation webapi.ConversationID,
	work func(context.Context, *sql.Tx) error,
) (bool, error) {
	return s.DB(conversation).Read(ctx, work)
}

// notifyLocked wakes operations waiting for writer availability or shutdown.
func (s *ConversationStores) notifyLocked() {
	close(s.changed)
	s.changed = make(chan struct{})
}

func (s *ConversationStores) begin() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closed {
		return ErrClosed
	}
	s.active++
	return nil
}

func (s *ConversationStores) end() {
	s.mu.Lock()
	s.active--
	s.notifyLocked()
	s.mu.Unlock()
}

// Close drains admitted operations and joins errors closing every writer.
// Once shutdown starts it completes even if ctx is canceled.
func (s *ConversationStores) Close(ctx context.Context) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	s.mu.Lock()
	s.closed = true
	s.notifyLocked()
	for s.active != 0 {
		changed := s.changed
		s.mu.Unlock()
		<-changed
		s.mu.Lock()
	}
	writers := s.writers
	s.writers = make(map[string]*writer)
	s.mu.Unlock()
	var err error
	for _, w := range writers {
		if w.db != nil {
			err = errors.Join(err, w.db.Close())
		}
	}
	return sqlError(err)
}

// acquire pins the writer through an operation; opening/eviction happen outside the mutex.
func (d *ConversationDB) acquire(ctx context.Context) (*writer, error) {
	s := d.stores
	for {
		s.mu.Lock()
		if s.closed {
			s.mu.Unlock()
			return nil, ErrClosed
		}
		if w := s.writers[d.file]; w != nil {
			if w.db == nil {
				ready := w.ready
				s.mu.Unlock()
				select {
				case <-ready:
					continue
				case <-ctx.Done():
					return nil, ctx.Err()
				}
			}
			s.tick++
			w.used = s.tick
			w.pins++
			s.mu.Unlock()
			return w, nil
		}
		var evicted *writer
		if len(s.writers) >= s.limit {
			var key string
			key, evicted = s.oldestWriterLocked()
			if evicted == nil {
				changed := s.changed
				s.mu.Unlock()
				select {
				case <-changed:
					continue
				case <-ctx.Done():
					return nil, ctx.Err()
				}
			}
			delete(s.writers, key)
		}
		w := &writer{ready: make(chan struct{}), pins: 1}
		s.writers[d.file] = w
		s.mu.Unlock()
		return d.openWriter(ctx, w, evicted)
	}
}

func (d *ConversationDB) release(w *writer) {
	s := d.stores
	s.mu.Lock()
	w.pins--
	s.notifyLocked()
	s.mu.Unlock()
}

// Call owns one writer transaction, committing work on success and rolling back on error.
func (d *ConversationDB) Call(ctx context.Context, work func(context.Context, *sql.Tx) error) error {
	return d.call(ctx, work, nil)
}

func (d *ConversationDB) call(
	ctx context.Context,
	work func(context.Context, *sql.Tx) error,
	commit func(context.Context, *sql.Tx) error,
) error {
	if err := d.stores.begin(); err != nil {
		return err
	}
	defer d.stores.end()
	w, err := d.acquire(ctx)
	if err != nil {
		return err
	}
	defer d.release(w)
	return transaction(ctx, w.db, work, commit)
}

var coldReads = make(chan struct{}, 64)

// Read uses a short-lived read-only connection, never opening a writer or creating a file.
func (d *ConversationDB) Read(ctx context.Context, work func(context.Context, *sql.Tx) error) (bool, error) {
	if err := d.stores.begin(); err != nil {
		return false, err
	}
	defer d.stores.end()
	select {
	case coldReads <- struct{}{}:
	case <-ctx.Done():
		return false, ctx.Err()
	}
	defer func() { <-coldReads }()
	path := filepath.Join(d.stores.directory, d.file+".sqlite")
	if _, err := os.Stat(path); err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return false, nil
		}
		return false, &Error{Kind: IOFailure, Err: err}
	}
	db, err := openSQLite(ctx, path, "", true)
	if err != nil {
		return false, err
	}
	found := false
	err = transaction(ctx, db, func(ctx context.Context, tx *sql.Tx) error {
		var version uint32
		if err := tx.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version); err != nil {
			return err
		}
		if version == 0 {
			return nil
		}
		if version != schemaVersion(conversationSchema) {
			return &Error{Kind: OtherSchema, Path: path}
		}
		found = true
		return work(ctx, tx)
	}, nil)
	return found, errors.Join(err, db.Close())
}

// oldestWriterLocked selects the least recently used writer that no operation pins.
func (s *ConversationStores) oldestWriterLocked() (string, *writer) {
	var evicted *writer
	var key string
	for name, w := range s.writers {
		if w.db != nil && w.pins == 0 && (evicted == nil || w.used < evicted.used) {
			key = name
			evicted = w
		}
	}
	return key, evicted
}

// openWriter completes the reserved writer opening and publishes its result to waiting operations.
func (d *ConversationDB) openWriter(ctx context.Context, w, evicted *writer) (*writer, error) {
	s := d.stores
	var db *sql.DB
	var err error
	if evicted != nil {
		err = evicted.db.Close()
	}
	if err == nil {
		db, err = openSQLite(ctx, filepath.Join(s.directory, d.file+".sqlite"), conversationSchema, false)
	}
	s.mu.Lock()
	if err != nil {
		delete(s.writers, d.file)
	} else {
		s.tick++
		w.used = s.tick
		w.db = db
	}
	close(w.ready)
	s.notifyLocked()
	s.mu.Unlock()
	if err != nil {
		return nil, sqlError(err)
	}
	return w, nil
}
