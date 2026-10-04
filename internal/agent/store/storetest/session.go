package storetest

import (
	"context"
	"errors"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/types"
)

type memorySession struct {
	tree *MemoryTreeStore
	id   types.NodeID
}

// Save waits outside the mutex, then checks the invocation guard at the commit.
func (s *memorySession) Save(ctx context.Context, update store.CheckpointUpdate, guard store.CommitGuard) error {
	saved, err := encodeUpdate(s.id, update)
	if err != nil {
		return err
	}
	s.tree.mu.Lock()
	gate := s.tree.saveHold
	s.tree.mu.Unlock()
	if err := gate.pass(ctx); err != nil {
		return err
	}
	s.tree.mu.Lock()
	defer s.tree.mu.Unlock()
	if err := guard.Check(); err != nil {
		return err
	}
	if s.tree.failingSaves > 0 {
		s.tree.failingSaves--
		return errors.New("the database refused the save")
	}
	return s.tree.applySaveLocked(saved)
}

// Load returns the decoded checkpoint, and false when the node is absent.
func (s *memorySession) Load(ctx context.Context) (store.Checkpoint, bool, error) {
	if err := ctx.Err(); err != nil {
		return store.Checkpoint{}, false, err
	}
	s.tree.mu.Lock()
	node, exists := s.tree.nodes[s.id]
	s.tree.mu.Unlock()
	if !exists {
		return store.Checkpoint{}, false, nil
	}
	checkpoint, err := node.rows.checkpoint(s.id)
	if err != nil || checkpoint == nil {
		return store.Checkpoint{}, false, err
	}
	return *checkpoint, true, nil
}

// Blobs returns the tree's shared blob namespace.
func (s *memorySession) Blobs() store.Blobs { return s.tree.blobs }
