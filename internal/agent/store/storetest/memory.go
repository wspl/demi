package storetest

// API checkpoint: named parameters document the interface until bodies are ported.
//revive:disable:unused-parameter

import (
	"context"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

// MemoryTreeStore realizes the tree contract with nothing durable, recording
// every save. It supports concurrent calls and returns owned snapshots.
type MemoryTreeStore struct{}

// NewMemoryTreeStore creates a store with its own in-memory blob namespace.
func NewMemoryTreeStore() *MemoryTreeStore { panic("not written: a-store") }

// NewMemoryTreeStoreWithBlobs creates a store using the supplied namespace.
func NewMemoryTreeStoreWithBlobs(blobs store.BlobStore) *MemoryTreeStore {
	panic("not written: a-store")
}

// Copy takes an independent copy of the stored state sharing its blob namespace.
func (s *MemoryTreeStore) Copy() *MemoryTreeStore { panic("not written: a-store") }

// KeepOutput records what the conversation holds of an ended command's output.
func (s *MemoryTreeStore) KeepOutput(command core.CommandID, output store.StoredOutput) {
	panic("not written: a-store")
}

// Save records the node and update of one successful save.
type Save struct {
	Node   core.NodeID
	Update store.CheckpointUpdate
}

// Saves returns successful saves in commit order.
func (s *MemoryTreeStore) Saves() []Save { panic("not written: a-store") }

// Checkpoint returns a node's checkpoint, or nil if absent.
func (s *MemoryTreeStore) Checkpoint(id core.NodeID) *store.Checkpoint { panic("not written: a-store") }

// Record returns a node's record, or nil if absent.
func (s *MemoryTreeStore) Record(id core.NodeID) *store.NodeRecord { panic("not written: a-store") }

// Numbered returns the node known by number, and whether it exists.
func (s *MemoryTreeStore) Numbered(number uint64) (core.NodeID, bool) { panic("not written: a-store") }

// FailSaves refuses the next count saves as a failing database would.
func (s *MemoryTreeStore) FailSaves(count int) { panic("not written: a-store") }

// HoldSaves holds subsequent saves until the returned gate is released.
// The acquiring test registers Release with t.Cleanup immediately.
func (s *MemoryTreeStore) HoldSaves() *StoreGate { panic("not written: a-store") }

// HoldChildrenOf holds subsequent children reads for parent until release.
// The acquiring test registers Release with t.Cleanup immediately.
func (s *MemoryTreeStore) HoldChildrenOf(parent core.NodeID) *StoreGate {
	panic("not written: a-store")
}

// Node returns a node's record, or nil when it does not exist.
func (s *MemoryTreeStore) Node(ctx context.Context, id core.NodeID) (*store.NodeRecord, error) {
	panic("not written: a-store")
}

// Children returns direct children in number order, live and archived alike.
func (s *MemoryTreeStore) Children(ctx context.Context, parent core.NodeID) ([]store.NodeRecord, error) {
	panic("not written: a-store")
}

// CreateNode commits the record and first checkpoint; an existing node is refused.
func (s *MemoryTreeStore) CreateNode(ctx context.Context, record store.NodeRecord, initial store.CheckpointUpdate) error {
	panic("not written: a-store")
}

// SessionStore returns the node's checkpoint store. Saves also mark carried
// child completions delivered in the same commit.
func (s *MemoryTreeStore) SessionStore(id core.NodeID) store.SessionStore {
	panic("not written: a-store")
}

// CloseNode closes a node after its final checkpoint, initially undelivered.
func (s *MemoryTreeStore) CloseNode(ctx context.Context, id core.NodeID, closed store.NodeClose) error {
	panic("not written: a-store")
}

// ReopenNode starts a new round and queues its reviving message atomically.
func (s *MemoryTreeStore) ReopenNode(ctx context.Context, id core.NodeID, round uint64, startedAt core.Timestamp, message core.QueuedMessage) error {
	panic("not written: a-store")
}

// MarkDelivered marks only the named current round delivered.
func (s *MemoryTreeStore) MarkDelivered(ctx context.Context, id core.NodeID, round uint64) error {
	panic("not written: a-store")
}

// DeleteNode deletes the node and all descendants with all their rows.
func (s *MemoryTreeStore) DeleteNode(ctx context.Context, id core.NodeID) error {
	panic("not written: a-store")
}

// NextNumber records the following number before returning this one.
func (s *MemoryTreeStore) NextNumber(ctx context.Context, sequence core.Sequence) (uint64, error) {
	panic("not written: a-store")
}

// CommandOutput returns an ended command's output record, or nil if unknown.
func (s *MemoryTreeStore) CommandOutput(ctx context.Context, command core.CommandID) (store.StoredOutput, error) {
	panic("not written: a-store")
}

var _ store.TreeStore = (*MemoryTreeStore)(nil)
