package storetest

import (
	"bytes"
	"context"
	"fmt"
	"maps"
	"slices"
	"sync"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/types"
)

// MemoryTreeStore realizes the tree contract with nothing durable, recording
// every save. It supports concurrent calls and returns owned snapshots.
type MemoryTreeStore struct {
	// mu protects atomic tree commits and test controls; gates wait outside it.
	mu            sync.Mutex
	nodes         map[types.NodeID]storedNode
	saves         []savedRows
	sequences     map[types.Sequence]uint64
	outputs       map[types.CommandID]store.StoredOutput
	failingSaves  int
	saveHold      *StoreGate
	childrenHolds map[types.NodeID]*StoreGate
	blobs         store.Blobs
}

// NewMemoryTreeStore creates a store with its own in-memory blob namespace.
func NewMemoryTreeStore() *MemoryTreeStore {
	return NewMemoryTreeStoreWithBlobs(NewMemoryBlobs())
}

// NewMemoryTreeStoreWithBlobs creates a store using the supplied namespace.
func NewMemoryTreeStoreWithBlobs(blobs store.Blobs) *MemoryTreeStore {
	return &MemoryTreeStore{
		nodes:         map[types.NodeID]storedNode{},
		sequences:     map[types.Sequence]uint64{},
		outputs:       map[types.CommandID]store.StoredOutput{},
		childrenHolds: map[types.NodeID]*StoreGate{},
		blobs:         blobs,
	}
}

// Copy takes an independent copy of the stored state sharing its blob namespace.
func (s *MemoryTreeStore) Copy() *MemoryTreeStore {
	s.mu.Lock()
	defer s.mu.Unlock()
	return &MemoryTreeStore{
		nodes:         maps.Clone(s.nodes),
		saves:         slices.Clone(s.saves),
		sequences:     maps.Clone(s.sequences),
		outputs:       maps.Clone(s.outputs),
		failingSaves:  s.failingSaves,
		saveHold:      s.saveHold,
		childrenHolds: maps.Clone(s.childrenHolds),
		blobs:         s.blobs,
	}
}

// KeepOutput records what the conversation holds of an ended command's output.
func (s *MemoryTreeStore) KeepOutput(command types.CommandID, output store.StoredOutput) {
	copied := copyOutput(output)
	s.mu.Lock()
	defer s.mu.Unlock()
	s.outputs[command] = copied
}

// Save records the node and update of one successful save.
type Save struct {
	// Node identifies the saved node.
	Node types.NodeID
	// Update contains the committed checkpoint changes.
	Update store.CheckpointUpdate
}

// Saves returns successful saves in commit order.
func (s *MemoryTreeStore) Saves() []Save {
	s.mu.Lock()
	saved := slices.Clone(s.saves)
	s.mu.Unlock()
	saves := make([]Save, 0, len(saved))
	for _, saved := range saved {
		// These immutable bytes were produced by generated encoders at admission.
		// Decoding them cannot fail; corrupt/incomplete transcript loads use Load.
		state, _ := store.DecodeCheckpointState(saved.rows.state)
		update := store.CheckpointUpdate{
			State:         state,
			BlockCount:    saved.rows.count,
			ChangedBlocks: []store.ChangedBlock{},
		}
		if saved.rows.command != nil {
			command, _ := store.DecodeCommandStateSnapshot(saved.rows.command)
			update.CommandState = &command
		}
		for _, index := range slices.Sorted(maps.Keys(saved.rows.blocks)) {
			block, _ := types.DecodeBlock(saved.rows.blocks[index])
			update.ChangedBlocks = append(update.ChangedBlocks, store.ChangedBlock{Index: index, Block: block})
		}
		saves = append(saves, Save{Node: saved.id, Update: update})
	}
	return saves
}

// Checkpoint returns a node's checkpoint, or nil if absent.
func (s *MemoryTreeStore) Checkpoint(id types.NodeID) *store.Checkpoint {
	s.mu.Lock()
	node, exists := s.nodes[id]
	s.mu.Unlock()
	if !exists {
		return nil
	}
	// A corrupt checkpoint reads as absent here; Load returns the corruption
	// error to tests that inspect those failures.
	checkpoint, _ := node.rows.checkpoint(id)
	return checkpoint
}

// Record returns a node's record, or nil if absent.
func (s *MemoryTreeStore) Record(id types.NodeID) *store.NodeRecord {
	s.mu.Lock()
	node, exists := s.nodes[id]
	s.mu.Unlock()
	if !exists {
		return nil
	}
	return new(copyRecord(node.record))
}

// Numbered returns the node known by number, and whether it exists.
func (s *MemoryTreeStore) Numbered(number uint64) (types.NodeID, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, id := range slices.Sorted(maps.Keys(s.nodes)) {
		if s.nodes[id].record.Number == number {
			return id, true
		}
	}
	return "", false
}

// FailSaves refuses the next count saves as a failing database would.
func (s *MemoryTreeStore) FailSaves(count int) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.failingSaves = count
}

// HoldSaves holds subsequent saves until the returned gate is released.
// The acquiring test registers Release with t.Cleanup immediately.
func (s *MemoryTreeStore) HoldSaves() *StoreGate {
	gate := &StoreGate{}
	s.mu.Lock()
	s.saveHold = gate
	s.mu.Unlock()
	return gate
}

// HoldChildrenOf holds subsequent children reads for parent until release.
// The acquiring test registers Release with t.Cleanup immediately.
func (s *MemoryTreeStore) HoldChildrenOf(parent types.NodeID) *StoreGate {
	gate := &StoreGate{}
	s.mu.Lock()
	s.childrenHolds[parent] = gate
	s.mu.Unlock()
	return gate
}

// Node returns a node's record, and false when it does not exist.
func (s *MemoryTreeStore) Node(ctx context.Context, id types.NodeID) (store.NodeRecord, bool, error) {
	if err := ctx.Err(); err != nil {
		return store.NodeRecord{}, false, err
	}
	record := s.Record(id)
	if record == nil {
		return store.NodeRecord{}, false, nil
	}
	return *record, true, nil
}

// Children returns direct children in number order, live and archived alike.
func (s *MemoryTreeStore) Children(ctx context.Context, parent types.NodeID) ([]store.NodeRecord, error) {
	s.mu.Lock()
	gate := s.childrenHolds[parent]
	s.mu.Unlock()
	if err := gate.pass(ctx); err != nil {
		return nil, err
	}
	s.mu.Lock()
	children := []store.NodeRecord{}
	for _, id := range slices.Sorted(maps.Keys(s.nodes)) {
		node := s.nodes[id]
		if node.record.Parent != nil && *node.record.Parent == parent {
			children = append(children, copyRecord(node.record))
		}
	}
	s.mu.Unlock()
	slices.SortStableFunc(children, func(a, b store.NodeRecord) int {
		if a.Number < b.Number {
			return -1
		}
		if a.Number > b.Number {
			return 1
		}
		return 0
	})
	return children, nil
}

// CreateNode commits the record and first checkpoint; an existing node is refused.
func (s *MemoryTreeStore) CreateNode(
	ctx context.Context,
	record store.NodeRecord,
	initial store.CheckpointUpdate,
) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	saved, err := encodeUpdate(record.ID, initial)
	if err != nil {
		return err
	}
	empty, err := store.InitialCommandState().MarshalJSON()
	if err != nil {
		return err
	}
	record = copyRecord(record)
	s.mu.Lock()
	defer s.mu.Unlock()
	if _, exists := s.nodes[record.ID]; exists {
		return fmt.Errorf("node %s already exists", record.ID)
	}
	s.nodes[record.ID] = storedNode{record: record, rows: checkpointRows{command: empty, blocks: map[int][]byte{}}}
	if err := s.applySaveLocked(saved); err != nil {
		delete(s.nodes, record.ID)
		return err
	}
	return nil
}

// Session returns the node's checkpoint store. Saves also mark carried
// child completions delivered in the same commit.
func (s *MemoryTreeStore) Session(id types.NodeID) store.Session {
	return &memorySession{tree: s, id: id}
}

// CloseNode closes a node after its final checkpoint, initially undelivered.
func (s *MemoryTreeStore) CloseNode(ctx context.Context, id types.NodeID, closed store.NodeClose) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	node, exists := s.nodes[id]
	if !exists {
		return missing(id)
	}
	node.record.Closed = &closed
	node.record = copyRecord(node.record)
	node.record.Delivered = false
	s.nodes[id] = node
	return nil
}

// ReopenNode starts a new round and queues its reviving message atomically.
func (s *MemoryTreeStore) ReopenNode(
	ctx context.Context,
	id types.NodeID,
	round uint64,
	startedAt types.Timestamp,
	message types.QueuedMessage,
) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	node, exists := s.nodes[id]
	if !exists {
		return missing(id)
	}
	state, err := store.DecodeCheckpointState(node.rows.state)
	if err != nil {
		return err
	}
	state.Queue = []types.QueuedMessage{message}
	encoded, err := state.MarshalJSON()
	if err != nil {
		return err
	}
	node.rows.state = encoded
	node.record.Closed = nil
	node.record.Delivered = false
	node.record.Round = round
	node.record.StartedAt = startedAt
	s.nodes[id] = node
	return nil
}

// MarkDelivered marks only the named current round delivered.
func (s *MemoryTreeStore) MarkDelivered(ctx context.Context, id types.NodeID, round uint64) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	node, exists := s.nodes[id]
	if !exists {
		return missing(id)
	}
	if node.record.Round == round {
		node.record.Delivered = true
		s.nodes[id] = node
	}
	return nil
}

// DeleteNode deletes the node and all descendants with all their rows.
func (s *MemoryTreeStore) DeleteNode(ctx context.Context, id types.NodeID) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	doomed := []types.NodeID{id}
	for index := 0; index < len(doomed); index++ {
		for child, node := range s.nodes {
			if node.record.Parent != nil && *node.record.Parent == doomed[index] {
				doomed = append(doomed, child)
			}
		}
		delete(s.nodes, doomed[index])
	}
	return nil
}

// NextNumber records the following number before returning this one.
func (s *MemoryTreeStore) NextNumber(ctx context.Context, sequence types.Sequence) (uint64, error) {
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	next, exists := s.sequences[sequence]
	if !exists {
		next = 1
	}
	s.sequences[sequence] = next + 1
	return next, nil
}

// CommandOutput returns an ended command's output record, or nil if unknown.
func (s *MemoryTreeStore) CommandOutput(ctx context.Context, command types.CommandID) (store.StoredOutput, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	s.mu.Lock()
	output := s.outputs[command]
	s.mu.Unlock()
	return copyOutput(output), nil
}

var _ store.Tree = (*MemoryTreeStore)(nil)

// copyOutput detaches kept command bytes and optional missing-output records.
func copyOutput(output store.StoredOutput) store.StoredOutput {
	switch output := output.(type) {
	case *store.OutputStored:
		copied := output.Output
		copied.Records = slices.Clone(copied.Records)
		for index := range copied.Records {
			record := &copied.Records[index]
			record.Bytes = bytes.Clone(record.Bytes)
			if record.LeftOut != nil {
				record.LeftOut = new(*record.LeftOut)
			}
		}
		if copied.Missing != nil {
			copied.Missing = new(*copied.Missing)
		}
		return &store.OutputStored{Output: copied}
	case *store.OutputNotStored:
		return &store.OutputNotStored{Reason: output.Reason}
	case *store.OutputRemoved:
		return &store.OutputRemoved{At: output.At}
	}
	return nil
}
