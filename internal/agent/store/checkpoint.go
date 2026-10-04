package store

import (
	"fmt"

	"github.com/wspl/demi/internal/types"
)

// Checkpoint is a node's checkpoint as the store gives it back.
type Checkpoint struct {
	// State holds the saved session state.
	State CheckpointState
	// Transcript contains the saved blocks in row order.
	Transcript []types.Block
	// CommandState holds the saved command versions and boundaries.
	CommandState CommandStateSnapshot
}

// ChangedBlock is a changed transcript row at Index.
type ChangedBlock struct {
	// Index is the transcript row to replace.
	Index int
	// Block is the replacement transcript block.
	Block types.Block
}

// CheckpointUpdate carries only what changed since the last save.
type CheckpointUpdate struct {
	// State is the session state to save.
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
func (u CheckpointUpdate) CarriedCompletions() ([]types.CompletionID, error) {
	messages := make([]types.AgentMessage, 0, len(u.State.AgentInputs))
	for _, input := range u.State.AgentInputs {
		messages = append(messages, input.Message)
	}
	for _, changed := range u.ChangedBlocks {
		if block, ok := changed.Block.(*types.AgentMessageBlock); ok {
			messages = append(messages, block.Message)
		}
	}
	rounds := []types.CompletionID{}
	seen := map[types.CompletionID]bool{}
	for _, message := range messages {
		if _, ok := message.Event.(*types.CompletionEvent); !ok {
			continue
		}
		round, err := types.ParseCompletionID(string(message.ID))
		if err != nil {
			return nil, fmt.Errorf("%w: %w", ErrCorrupt, err)
		}
		if !seen[round] {
			seen[round] = true
			rounds = append(rounds, round)
		}
	}
	return rounds, nil
}
