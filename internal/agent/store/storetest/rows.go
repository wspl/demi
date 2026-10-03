package storetest

import (
	"fmt"
	"maps"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

// checkpointRows keeps immutable encoded rows, so callers cannot mutate a saved
// checkpoint through a shared pointer. Decoding always uses generated codecs.
type checkpointRows struct {
	state   []byte
	command []byte
	blocks  map[int][]byte
	count   int
}
type savedRows struct {
	id          core.NodeID
	rows        checkpointRows
	completions []core.CompletionID
}
type storedNode struct {
	record store.NodeRecord
	rows   checkpointRows
}

// encodeUpdate captures a save's owned rows before acquiring the store lock.
func encodeUpdate(id core.NodeID, update store.CheckpointUpdate) (savedRows, error) {
	saved := savedRows{id: id, rows: checkpointRows{blocks: map[int][]byte{}, count: update.BlockCount}}
	var err error
	saved.completions, err = update.CarriedCompletions()
	if err != nil {
		return savedRows{}, err
	}
	saved.rows.state, err = update.State.MarshalJSON()
	if err != nil {
		return savedRows{}, err
	}
	if update.CommandState != nil {
		saved.rows.command, err = update.CommandState.MarshalJSON()
		if err != nil {
			return savedRows{}, err
		}
	}
	for _, changed := range update.ChangedBlocks {
		encoded, err := (core.BlockJSON{Value: changed.Block}).MarshalJSON()
		if err != nil {
			return savedRows{}, err
		}
		saved.rows.blocks[changed.Index] = encoded
	}
	return saved, nil
}

// checkpoint decodes and checks the complete saved node, refusing missing rows.
func (r checkpointRows) checkpoint(id core.NodeID) (*store.Checkpoint, error) {
	state, err := store.DecodeCheckpointState(r.state)
	if err != nil {
		return nil, &store.Error{Kind: store.Corrupt, Message: err.Error(), Cause: err}
	}
	command, err := store.DecodeCommandStateSnapshot(r.command)
	if err != nil {
		return nil, &store.Error{Kind: store.Corrupt, Message: err.Error(), Cause: err}
	}
	if _, err = store.RestoreCommandStateHistory(command); err != nil {
		return nil, &store.Error{Kind: store.Corrupt, Message: err.Error(), Cause: err}
	}
	blocks := make([]core.Block, 0, r.count)
	for index := 0; index < r.count; index++ {
		data, exists := r.blocks[index]
		if !exists {
			return nil, &store.Error{
				Kind:    store.Corrupt,
				Message: fmt.Sprintf("node %s has no block row %d", id, index),
			}
		}
		block, err := core.DecodeBlock(data)
		if err != nil {
			return nil, &store.Error{Kind: store.Corrupt, Message: err.Error(), Cause: err}
		}
		blocks = append(blocks, block)
	}
	return &store.Checkpoint{State: state, Transcript: blocks, CommandState: command}, nil
}

// applySaveLocked publishes all node rows and carried completion deliveries together.
// The caller holds the tree mutex; no IO or wait occurs here.
func (s *MemoryTreeStore) applySaveLocked(saved savedRows) error {
	node, exists := s.nodes[saved.id]
	if !exists {
		return missing(saved.id)
	}
	next := node
	next.rows.blocks = maps.Clone(node.rows.blocks)
	if saved.rows.command != nil {
		if err := checkCommandVersions(node.rows.command, saved.rows.command); err != nil {
			return err
		}
		next.rows.command = saved.rows.command
	}
	for index, block := range saved.rows.blocks {
		next.rows.blocks[index] = block
	}
	for index := range next.rows.blocks {
		if index >= saved.rows.count {
			delete(next.rows.blocks, index)
		}
	}
	next.rows.state = saved.rows.state
	next.rows.count = saved.rows.count
	s.nodes[saved.id] = next
	for _, round := range saved.completions {
		child, exists := s.nodes[round.Child]
		if exists && child.record.Parent != nil && *child.record.Parent == saved.id &&
			child.record.Round == round.Round {
			child.record.Delivered = true
			s.nodes[round.Child] = child
		}
	}
	s.saves = append(s.saves, saved)
	return nil
}

// missing describes an operation on an absent tree node.
func missing(id core.NodeID) error {
	return &store.Error{Kind: store.OperationFailed, Message: fmt.Sprintf("no node %s", id)}
}

// copyRecord detaches the optional identity and close payloads of a tree record.
func copyRecord(record store.NodeRecord) store.NodeRecord {
	if record.Parent != nil {
		record.Parent = new(*record.Parent)
	}
	if record.Profile != nil {
		record.Profile = new(*record.Profile)
	}
	if record.Closed != nil {
		closed := *record.Closed
		switch phase := closed.Phase.(type) {
		case *store.Completed:
			closed.Phase = &store.Completed{Result: phase.Result}
		case *store.Aborted:
			closed.Phase = &store.Aborted{}
		case *store.Failed:
			closed.Phase = &store.Failed{Failure: phase.Failure}
		}
		record.Closed = &closed
	}
	return record
}

func checkCommandVersions(current, next []byte) error {
	previous, err := store.DecodeCommandStateSnapshot(current)
	if err != nil {
		return err
	}
	proposed, err := store.DecodeCommandStateSnapshot(next)
	if err != nil {
		return err
	}
	for _, version := range proposed.Versions {
		for _, old := range previous.Versions {
			if old.Revision != version.Revision {
				continue
			}
			// CommandStateHistory owns canonical JSON equality, including number form.
			snapshot := previous
			snapshot.Revision = old.Revision
			history, err := store.RestoreCommandStateHistory(snapshot)
			if err != nil {
				return err
			}
			change, err := history.Prepare(version.Values)
			if err != nil {
				return err
			}
			if change != nil {
				return &store.Error{
					Kind:    store.OperationFailed,
					Message: fmt.Sprintf("command-state version %d is immutable", version.Revision),
				}
			}
		}
	}
	return nil
}
