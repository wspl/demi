package database

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

// TreeStore is one conversation's agent tree in its database, with its owner's blobs.
type TreeStore struct{}

// NewTreeStore binds a conversation database, its owner's blobs and commit callback.
func NewTreeStore(db *ConversationDB, blobs OwnerBlobs, saved Saved) *TreeStore {
	panic("not written: b-database")
}

var _ store.TreeStore = (*TreeStore)(nil)

// Node returns a node's record, or nil when it does not exist.
func (s *TreeStore) Node(ctx context.Context, id core.NodeID) (*store.NodeRecord, error) {
	panic("not written: b-database")
}

// Children returns direct children in number order, live and archived alike.
func (s *TreeStore) Children(ctx context.Context, parent core.NodeID) ([]store.NodeRecord, error) {
	panic("not written: b-database")
}

// CreateNode commits the record and first checkpoint; an existing node is refused.
func (s *TreeStore) CreateNode(ctx context.Context, record store.NodeRecord, initial store.CheckpointUpdate) error {
	panic("not written: b-database")
}

// SessionStore returns the node's checkpoint store. Saves also mark carried
// child completions delivered in the same commit.
func (s *TreeStore) SessionStore(id core.NodeID) store.SessionStore { panic("not written: b-database") }

// CloseNode closes a node after its final checkpoint, initially undelivered.
func (s *TreeStore) CloseNode(ctx context.Context, id core.NodeID, closed store.NodeClose) error {
	panic("not written: b-database")
}

// ReopenNode starts a new round and queues its reviving message atomically.
func (s *TreeStore) ReopenNode(ctx context.Context, id core.NodeID, round uint64, startedAt core.Timestamp, message core.QueuedMessage) error {
	panic("not written: b-database")
}

// MarkDelivered marks only the named current round delivered.
func (s *TreeStore) MarkDelivered(ctx context.Context, id core.NodeID, round uint64) error {
	panic("not written: b-database")
}

// DeleteNode deletes the node and all descendants with all their rows.
func (s *TreeStore) DeleteNode(ctx context.Context, id core.NodeID) error {
	panic("not written: b-database")
}

// NextNumber records the following number before returning this one.
func (s *TreeStore) NextNumber(ctx context.Context, sequence core.Sequence) (uint64, error) {
	panic("not written: b-database")
}

// CommandOutput returns an ended command's output record, or nil if unknown.
func (s *TreeStore) CommandOutput(ctx context.Context, command core.CommandID) (store.StoredOutput, error) {
	panic("not written: b-database")
}
