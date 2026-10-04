package database

import (
	"context"
	"database/sql"
	"sync"
)

// CommitPoint is the checkpoint's commit, optionally held by a test.
type CommitPoint struct{ hold *CommitHold }

// CommitPoint captures the current test hold.
func (d *ConversationDB) CommitPoint() CommitPoint {
	d.stores.mu.Lock()
	defer d.stores.mu.Unlock()
	return CommitPoint{hold: d.stores.hold}
}

// Commit waits for its captured hold and commits the transaction.
func (p CommitPoint) Commit(ctx context.Context, tx *sql.Tx) error {
	if p.hold != nil {
		if err := p.hold.pass(ctx); err != nil {
			return err
		}
	}
	return tx.Commit()
}

// CommitHold stops checkpoints immediately before commit; its owner releases it.
type CommitHold struct {
	mu       sync.Mutex
	waiting  int
	changed  chan struct{}
	released chan struct{}
	once     sync.Once
}

// HoldCommits holds new checkpoints; tests use databasetest for cleanup ownership.
func (s *ConversationStores) HoldCommits() *CommitHold {
	h := &CommitHold{changed: make(chan struct{}), released: make(chan struct{})}
	s.mu.Lock()
	s.hold = h
	s.mu.Unlock()
	return h
}

// UntilWaiting waits for count simultaneous checkpoints or cancellation.
func (h *CommitHold) UntilWaiting(ctx context.Context, count int) error {
	for {
		h.mu.Lock()
		if h.waiting >= count {
			h.mu.Unlock()
			return nil
		}
		changed := h.changed
		h.mu.Unlock()
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-changed:
		}
	}
}

// Release lets current and later checkpoints through, once.
func (h *CommitHold) Release() {
	h.once.Do(func() {
		close(h.released)
	})
}

func (h *CommitHold) pass(ctx context.Context) error {
	h.mu.Lock()
	h.waiting++
	close(h.changed)
	h.changed = make(chan struct{})
	h.mu.Unlock()
	defer func() {
		h.mu.Lock()
		h.waiting--
		close(h.changed)
		h.changed = make(chan struct{})
		h.mu.Unlock()
	}()
	select {
	case <-h.released:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}
