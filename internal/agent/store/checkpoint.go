package store

// API checkpoint: named parameters document the interface until bodies are ported.
//revive:disable:unused-parameter

import "github.com/wspl/demi/internal/core"

// Checkpoint is a node's checkpoint as the store gives it back.
type Checkpoint struct {
	State        CheckpointState
	Transcript   []core.Block
	CommandState CommandStateSnapshot
}

// ChangedBlock is a changed transcript row at Index.
type ChangedBlock struct {
	Index int
	Block core.Block
}

// CheckpointUpdate carries only what changed since the last save.
type CheckpointUpdate struct {
	State CheckpointState
	// CommandState is present when changed; a new node carries version zero.
	CommandState *CommandStateSnapshot
	// ChangedBlocks holds changed rows in ascending index order.
	ChangedBlocks []ChangedBlock
	// BlockCount truncates rows at or beyond this index.
	BlockCount int
}

// CarriedCompletions returns the distinct child rounds carried by waiting
// input or changed agent-message blocks, which this save marks delivered.
func (u CheckpointUpdate) CarriedCompletions() ([]core.CompletionID, error) {
	panic("not written: a-store")
}
