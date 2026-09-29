package storage

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/wspl/demi/go/webapi"
)

const MaxWriters = 64

type writerEntry struct {
	db      *sql.DB
	ready   chan struct{}
	err     error
	used    uint64
	refs    int
	evicted bool
}
type ConversationStores struct {
	directory string
	limit     int
	mu        sync.Mutex
	writers   map[string]*writerEntry
	tick      uint64
	closed    bool
	opening   sync.WaitGroup
	active    sync.WaitGroup
}
type ConversationDB struct {
	stores *ConversationStores
	file   string
}

func OpenConversationStores(directory string, maxWriters int) (*ConversationStores, error) {
	if maxWriters < 1 {
		return nil, errors.New("the writer limit must be positive")
	}
	if err := os.MkdirAll(directory, 0755); err != nil {
		return nil, fmt.Errorf("the file system failed: %w", err)
	}
	return &ConversationStores{directory: directory, limit: maxWriters, writers: make(map[string]*writerEntry)}, nil
}
func (s *ConversationStores) DB(id webapi.ConversationID) *ConversationDB {
	return &ConversationDB{s, strings.ToLower(id.String())}
}
func (d *ConversationDB) path() string { return filepath.Join(d.stores.directory, d.file+".sqlite") }
func (d *ConversationDB) writer(ctx context.Context) (*sql.DB, func(), error) {
	s := d.stores
	for {
		s.mu.Lock()
		if s.closed {
			s.mu.Unlock()
			return nil, nil, ErrClosed
		}
		entry := s.writers[d.file]
		if entry != nil {
			if entry.ready != nil {
				ready := entry.ready
				s.mu.Unlock()
				select {
				case <-ready:
				case <-ctx.Done():
					return nil, nil, ctx.Err()
				}
				if entry.err != nil {
					return nil, nil, entry.err
				}
				continue
			}
			s.tick++
			entry.used = s.tick
			entry.refs++
			s.active.Add(1)
			s.mu.Unlock()
			return entry.db, func() { d.release(entry) }, nil
		}
		entry = &writerEntry{ready: make(chan struct{})}
		s.writers[d.file] = entry
		s.opening.Add(1)
		s.mu.Unlock()
		db, err := openDatabase(ctx, d.path(), conversationV1)
		s.mu.Lock()
		entry.err = err
		ready := entry.ready
		entry.ready = nil
		var evicted *sql.DB
		if err != nil {
			delete(s.writers, d.file)
		} else {
			entry.db = db
			s.tick++
			entry.used = s.tick
			if s.closed {
				entry.err = ErrClosed
				delete(s.writers, d.file)
				evicted = db
			} else {
				count := 0
				var oldest *writerEntry
				var oldestKey string
				for key, item := range s.writers {
					if item.db == nil {
						continue
					}
					count++
					if item != entry && (oldest == nil || item.used < oldest.used) {
						oldest = item
						oldestKey = key
					}
				}
				if count > s.limit && oldest != nil {
					delete(s.writers, oldestKey)
					oldest.evicted = true
					if oldest.refs == 0 {
						evicted = oldest.db
					}
				}
			}
		}
		close(ready)
		err = entry.err
		s.mu.Unlock()
		if evicted != nil {
			err = errors.Join(err, sqliteError(evicted.Close()))
		}
		s.opening.Done()
		if err != nil {
			return nil, nil, err
		}
	}
}
func (d *ConversationDB) release(entry *writerEntry) {
	defer d.stores.active.Done()
	s := d.stores
	s.mu.Lock()
	entry.refs--
	closeNow := entry.evicted && entry.refs == 0
	s.mu.Unlock()
	if closeNow {
		_ = entry.db.Close()
	} // The operation reports its statement/commit error; eviction is best effort.
}
func (s *ConversationStores) Close() error {
	s.mu.Lock()
	s.closed = true
	dbs := make([]*sql.DB, 0, len(s.writers))
	for _, entry := range s.writers {
		if entry.db != nil {
			dbs = append(dbs, entry.db)
		}
	}
	s.writers = make(map[string]*writerEntry)
	s.mu.Unlock()
	s.opening.Wait()
	s.active.Wait()
	var err error
	for _, db := range dbs {
		err = errors.Join(err, sqliteError(db.Close()))
	}
	return err
}

// read opens only existing files, and every callback observes one SQLite snapshot.
func (d *ConversationDB) read(ctx context.Context, work func(*sql.Tx) error) (bool, error) {
	s := d.stores
	s.mu.Lock()
	closed := s.closed
	s.mu.Unlock()
	if closed {
		return false, ErrClosed
	}
	if _, err := os.Stat(d.path()); err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return false, nil
		}
		return false, fmt.Errorf("the file system failed: %w", err)
	}
	location := url.URL{Scheme: "file", Path: d.path()}
	query := url.Values{"mode": {"ro"}, "_pragma": {"foreign_keys(1)", "busy_timeout(5000)"}}
	location.RawQuery = query.Encode()
	db, err := sql.Open("sqlite", location.String())
	if err != nil {
		return false, sqliteError(err)
	}
	defer db.Close()
	db.SetMaxOpenConns(1)
	tx, err := db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return false, sqliteError(err)
	}
	defer tx.Rollback()
	if err = work(tx); err != nil {
		return true, err
	}
	return true, sqliteError(tx.Commit())
}
func (d *ConversationDB) transaction(ctx context.Context, work func(*sql.Tx) error) error {
	db, release, err := d.writer(ctx)
	if err != nil {
		return err
	}
	defer release()
	tx, err := db.BeginTx(ctx, nil)
	if err != nil {
		return sqliteError(err)
	}
	defer tx.Rollback()
	if err = work(tx); err != nil {
		return err
	}
	return sqliteError(tx.Commit())
}
