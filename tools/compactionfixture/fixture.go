package main

import (
	"bytes"
	"compress/gzip"
	_ "embed"
	"fmt"
	"io"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
)

//go:generate go run github.com/wspl/demi/tools/contractgen

// The committed fixture: a session that compacted several times, in the
// agent's block format.
// +demi:root
// +demi:strict
type fixture struct {
	CWD         string       `json:"cwd"`
	BuiltTokens uint64       `json:"builtTokens"`
	Generations uint32       `json:"generations"`
	Blocks      []core.Block `json:"blocks"`
}

//go:embed testdata/large-context-fixture.json.gz
var compressedFixture []byte

func loadFixture() (fixture, error) {
	reader, err := gzip.NewReader(bytes.NewReader(compressedFixture))
	if err != nil {
		return fixture{}, fmt.Errorf("large-context-fixture.json.gz: %w", err)
	}
	defer func() { _ = reader.Close() }() // The reader owns only in-memory decompression state.
	data, err := io.ReadAll(reader)
	if err != nil {
		return fixture{}, fmt.Errorf("large-context-fixture.json.gz: %w", err)
	}
	value, err := decodeFixture(data)
	if err != nil {
		return fixture{}, fmt.Errorf("large-context-fixture.json.gz: %w", err)
	}
	return value, nil
}

// initialCheckpoint opens the recorded transcript with no queued work or command state.
func initialCheckpoint(f fixture, model core.ModelSelection) store.CheckpointUpdate {
	initial := store.CheckpointUpdate{
		State: store.CheckpointState{
			Phase: core.SessionPhaseIdle, CWD: f.CWD, Model: model,
			Queue: []core.QueuedMessage{}, AgentInputs: []store.PendingAgentInput{},
			Wakeups: []store.ScheduledWakeup{}, Edits: []store.EditReceipt{},
		},
		CommandState: new(store.InitialCommandState()), BlockCount: len(f.Blocks),
	}
	for i, block := range f.Blocks {
		initial.ChangedBlocks = append(initial.ChangedBlocks, store.ChangedBlock{Index: i, Block: block})
	}
	return initial
}
