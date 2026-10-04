package store_test

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/types"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

func TestStoredHistoryIsRefusedRatherThanRepaired(t *testing.T) {
	duplicate := store.InitialCommandState()
	duplicate.Boundaries = []store.SessionBoundary{
		{BlockID: "b1", Edge: store.AfterBlock},
		{BlockID: "b1", Edge: store.AfterBlock},
	}
	dangling := store.InitialCommandState()
	dangling.Boundaries = []store.SessionBoundary{{BlockID: "b1", Edge: store.BeforeUser, CommandRevision: 3}}
	current := store.InitialCommandState()
	current.Revision = 2
	noEmpty := store.InitialCommandState()
	noEmpty.Versions[0].Values["todos.json"] = json.RawMessage(`null`)
	repeated := store.InitialCommandState()
	repeated.Versions = append(repeated.Versions, repeated.Versions[0])
	for _, scenario := range []struct {
		name     string
		snapshot store.CommandStateSnapshot
	}{
		{
			"duplicate boundary",
			duplicate,
		},
		{
			"dangling boundary",
			dangling,
		},
		{
			"current version missing",
			current,
		},
		{
			"nonempty zero",
			noEmpty,
		},
		{
			"repeated version",
			repeated,
		},
		{
			"missing zero",
			store.CommandStateSnapshot{},
		},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			_, err := store.RestoreCommandStateHistory(scenario.snapshot)
			if err == nil || !strings.HasPrefix(err.Error(), "invalid command state: ") {
				t.Fatalf("invalid history: %v", err)
			}
		})
	}
	for _, key := range []string{"", "/etc", "C:\\x", "a/../b", "a\x00b", "a\\..\\b"} {
		if _, err := store.ParseCommandStorageKey(key); err == nil {
			t.Fatalf("invalid key %q accepted", key)
		}
	}
	if _, err := store.ParseCommandStorageKey("todos.json"); err != nil {
		t.Fatal(err)
	}
}

func TestCommandHistoryVersionsFollowCutsAndFailedSaves(t *testing.T) {
	history := store.NewCommandStateHistory()
	if history.TakeUpdate(nil) != nil {
		t.Fatal("new history is dirty")
	}
	values := map[store.CommandStorageKey]json.RawMessage{"todos.json": json.RawMessage(`{"a":1,"b":2}`)}
	first, err := history.Prepare(values)
	if err != nil || first == nil || first.Revision != 1 {
		t.Fatalf("first revision: %v %v", first, err)
	}
	// Pending state belongs to the transaction before Accept changes live reads.
	pending := history.TakeUpdate(first)
	if pending.Revision != 1 || history.Revision() != 0 {
		t.Fatal("pending version became current early")
	}
	history.Accept(*first)
	values["todos.json"][0] = '['
	owned := history.Values()
	owned["todos.json"][0] = '['
	same, err := history.Prepare(
		map[store.CommandStorageKey]json.RawMessage{"todos.json": json.RawMessage(`{"b":2.0,"a":1e0}`)},
	)
	if err != nil || same != nil {
		t.Fatalf("canonical equality or ownership: %v %v", same, err)
	}
	if _, err := history.Prepare(
		map[store.CommandStorageKey]json.RawMessage{"todos.json": json.RawMessage(`{broken`)},
	); err == nil {
		t.Fatal("invalid JSON accepted")
	}
	history.Capture("b1", store.BeforeUser, 0)
	history.Capture("b1", store.BeforeUser, 1)
	history.Capture("b1", store.AfterBlock, 0)
	history.Capture("b1", store.AfterBlock, 1)
	history.Capture("b2", store.AfterAssistant, 1)
	if before, ok := history.Boundary("b1", store.BeforeUser); !ok || before != 0 {
		t.Fatal("before-user boundary moved")
	}
	if after, _ := history.Boundary("b1", store.AfterBlock); after != 1 {
		t.Fatal("after-block boundary did not move")
	}
	update := history.TakeUpdate(nil)
	if update == nil || history.TakeUpdate(nil) != nil {
		t.Fatal("dirty checkpoint not taken once")
	}
	history.MarkDirty()
	if !reflect.DeepEqual(history.TakeUpdate(nil), update) {
		t.Fatal("failed save was not retried")
	}
	second, err := history.Prepare(map[store.CommandStorageKey]json.RawMessage{"todos.json": json.RawMessage(`true`)})
	if err != nil {
		t.Fatal(err)
	}
	history.Accept(*second)
	fork := history.Select([]types.Block{&types.UserBlock{BlockID: "b1"}}, 1, true)
	if fork.Revision != 1 || len(fork.Versions) != 2 || len(fork.Boundaries) != 2 {
		t.Fatalf("fork state: %#v", fork)
	}
	cut := history.Select([]types.Block{}, 0, false)
	restored, err := store.RestoreCommandStateHistory(cut)
	if err != nil {
		t.Fatal(err)
	}
	next, err := restored.Prepare(map[store.CommandStorageKey]json.RawMessage{"todos.json": json.RawMessage(`true`)})
	if err != nil || next.Revision != 3 {
		t.Fatalf("rewind reused revision: %v %v", next, err)
	}
	if history.HasBoundary("unknown", store.AfterBlock) {
		t.Fatal("unknown boundary exists")
	}
	encoded, err := history.Snapshot(nil).MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := store.DecodeCommandStateSnapshot(encoded)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(decoded, history.Snapshot(nil)) {
		t.Fatal("history codec changed state")
	}
}
