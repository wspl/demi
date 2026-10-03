package storetest

import (
	"context"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

type memorySession struct {
	tree *MemoryTreeStore
	id   core.NodeID
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
		return &store.Error{Kind: store.OperationFailed, Message: "the database refused the save"}
	}
	return s.tree.applySaveLocked(saved)
}

// Load returns the decoded checkpoint, or nil when the node is absent.
func (s *memorySession) Load(ctx context.Context) (*store.Checkpoint, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	s.tree.mu.Lock()
	node, exists := s.tree.nodes[s.id]
	s.tree.mu.Unlock()
	if !exists {
		return nil, nil
	}
	return node.rows.checkpoint(s.id)
}

// Blobs returns the tree's shared blob namespace.
func (s *memorySession) Blobs() store.BlobStore { return s.tree.blobs }
