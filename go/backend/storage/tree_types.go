package storage

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"strings"

	"github.com/wspl/demi/go/backend/storage/agentdata"
	"github.com/wspl/demi/go/core"
)

// These records preserve the Rust agent's storage encoding until G7d owns its
// runtime interfaces. They are data, not a second implementation of the agent.
type CheckpointState = agentdata.CheckpointState
type NodeRecord struct {
	ID                core.NodeID
	Number            uint64
	Parent            *core.NodeID
	Description       string
	Profile           *string
	Round             uint64
	StartedAt         core.Timestamp
	CanSpawnSubagents bool
	Closed            *NodeClose
	Delivered         bool
}
type NodeClose struct {
	Phase           string
	At              core.Timestamp
	Result, Failure *string
}
type CommandVersion struct {
	Revision uint64
	Values   map[string]jsontext.Value
}
type SessionBoundary struct {
	BlockID         core.BlockID
	Edge            string
	CommandRevision uint64
}
type CommandStateSnapshot struct {
	Revision   uint64
	Versions   []CommandVersion
	Boundaries []SessionBoundary
}
type Checkpoint struct {
	State        CheckpointState
	Transcript   []core.Block
	CommandState CommandStateSnapshot
}
type BlockChange struct {
	Index int
	Block core.Block
}
type CheckpointUpdate struct {
	State         CheckpointState
	CommandState  *CommandStateSnapshot
	ChangedBlocks []BlockChange
	BlockCount    int
}

func initialCommandState() CommandStateSnapshot {
	return CommandStateSnapshot{Versions: []CommandVersion{{Values: map[string]jsontext.Value{}}}, Boundaries: []SessionBoundary{}}
}
func decodeState(text string) (CheckpointState, error) {
	var value CheckpointState
	err := json.Unmarshal([]byte(text), &value)
	if err == nil {
		err = agentdata.Validate(value)
	}
	return value, corrupt("nodes", "state", err)
}
func commandValues(text string) (map[string]jsontext.Value, error) {
	var values map[string]jsontext.Value
	if err := json.Unmarshal([]byte(text), &values); err != nil {
		return nil, corrupt("command_snapshots", "entries", err)
	}
	if values == nil {
		return nil, &CorruptError{"command_snapshots", "entries", "expected an object"}
	}
	for key := range values {
		drive := len(key) >= 3 && ((key[0] >= 'a' && key[0] <= 'z') || (key[0] >= 'A' && key[0] <= 'Z')) && key[1] == ':' && (key[2] == '/' || key[2] == '\\')
		traverses := false
		for _, part := range strings.FieldsFunc(key, func(r rune) bool { return r == '/' || r == '\\' }) {
			if part == ".." {
				traverses = true
			}
		}
		if key == "" || strings.ContainsRune(key, 0) || strings.HasPrefix(key, "/") || drive || traverses {
			return nil, &CorruptError{"command_snapshots", "entries", fmt.Sprintf("command storage key %q is not a relative name without path traversal", key)}
		}
	}
	return values, nil
}
func (u CheckpointUpdate) completions() ([]core.CompletionID, error) {
	messages := make([]core.AgentMessage, 0, len(u.State.AgentInputs))
	for _, input := range u.State.AgentInputs {
		messages = append(messages, input.Message)
	}
	for _, change := range u.ChangedBlocks {
		if b, ok := change.Block.(core.BlockAgentMessage); ok {
			messages = append(messages, b.Message)
		}
	}
	rounds := []core.CompletionID{}
	seen := map[core.CompletionID]bool{}
	for _, message := range messages {
		if _, ok := message.Event.(core.AgentMessageEventCompletion); !ok {
			continue
		}
		round, err := core.ParseCompletionID(message.ID.String())
		if err != nil {
			return nil, err
		}
		if !seen[round] {
			seen[round] = true
			rounds = append(rounds, round)
		}
	}
	return rounds, nil
}
