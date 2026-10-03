package storetest

import (
	"context"
	"errors"
	"reflect"
	"sync"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }
func TestStoreContract(t *testing.T) {
	StoreContract(t, func(_ *testing.T) store.TreeStore { return NewMemoryTreeStore() })
}

func TestGuardAndCancellationLeaveCheckpointsUnchanged(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var workers sync.WaitGroup
		t.Cleanup(workers.Wait)
		memory := NewMemoryTreeStore()
		c := contractTree{t: t, store: memory}
		c.create(t.Context(), contractRecord("root", nil, 0), contractUpdate(nil, nil))
		original := c.load(t.Context(), "root")
		gate := memory.HoldSaves()
		t.Cleanup(gate.Release)
		lifetime, invalidate := context.WithCancel(t.Context())
		defer invalidate()
		result := make(chan error, 1)
		update := contractUpdate([]core.QueuedMessage{contractMessage("later")}, nil)
		workers.Go(func() {
			result <- memory.SessionStore("root").Save(t.Context(), update, store.NewCommitGuard(lifetime))
		})
		if err := gate.Wait(t.Context(), 1); err != nil {
			t.Fatal(err)
		}
		invalidate()
		gate.Release()
		if err := <-result; !errors.Is(err, store.ErrInvalidated) {
			t.Fatalf("guard result: %v", err)
		}
		gate = memory.HoldSaves()
		t.Cleanup(gate.Release)
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		go func() { result <- memory.SessionStore("root").Save(ctx, update, store.CommitGuard{}) }()
		if err := gate.Wait(t.Context(), 1); err != nil {
			t.Fatal(err)
		}
		cancel()
		if err := <-result; !errors.Is(err, context.Canceled) {
			t.Fatalf("cancel result: %v", err)
		}
		if gate.Waiting() != 0 {
			t.Fatal("canceled save still held")
		}
		if !reflect.DeepEqual(c.load(t.Context(), "root"), original) {
			t.Fatal("failed saves changed checkpoint")
		}
	})
}

func TestFailedCreationAndSaveAreAtomic(t *testing.T) {
	memory := NewMemoryTreeStore()
	c := contractTree{t: t, store: memory}
	invalid := contractUpdate(nil, nil)
	invalid.CommandState.Versions[0].Values["todos.json"] = []byte(`true`)
	if err := memory.CreateNode(t.Context(), contractRecord("root", nil, 0), invalid); err == nil {
		t.Fatal("initial immutable version was overwritten")
	}
	if memory.Record("root") != nil || len(memory.Saves()) != 0 {
		t.Fatal("failed create left rows")
	}
	c.create(t.Context(), contractRecord("root", nil, 0), contractUpdate(nil, nil))
	before := memory.Checkpoint("root")
	memory.FailSaves(1)
	update := contractUpdate([]core.QueuedMessage{contractMessage("later")}, nil)
	if err := memory.SessionStore("root").Save(t.Context(), update, store.CommitGuard{}); err == nil {
		t.Fatal("injected database failure did not fail")
	}
	if !reflect.DeepEqual(memory.Checkpoint("root"), before) {
		t.Fatal("failed save wrote rows")
	}
	c.save(t.Context(), "root", update)
	copied := memory.Copy()
	update.State.Queue[0].Content[0].(*core.UserText).Text = "mutated"
	loaded := memory.Checkpoint("root")
	loaded.State.Queue[0].Content[0].(*core.UserText).Text = "also mutated"
	if text := copied.Checkpoint("root").State.Queue[0].Content[0].(*core.UserText).Text; text != "brief" {
		t.Fatalf("snapshot aliases caller: %s", text)
	}
	if text := memory.Checkpoint("root").State.Queue[0].Content[0].(*core.UserText).Text; text != "brief" {
		t.Fatalf("load aliases store: %s", text)
	}
	c.save(t.Context(), "root", contractUpdate(nil, nil))
	if len(copied.Checkpoint("root").State.Queue) != 1 {
		t.Fatal("copy changed with source")
	}
}

func TestCorruptTranscriptStopsLoad(t *testing.T) {
	memory := NewMemoryTreeStore()
	update := contractUpdate(nil, nil)
	update.BlockCount = 1
	if err := memory.CreateNode(t.Context(), contractRecord("root", nil, 0), update); err != nil {
		t.Fatal(err)
	}
	_, _, err := memory.SessionStore("root").Load(t.Context())
	if !errors.Is(err, store.ErrCorrupt) {
		t.Fatalf("missing row: %v", err)
	}
}

func TestCommandOutputRecordsSurviveCopyAndOwnTheirBytes(t *testing.T) {
	memory := NewMemoryTreeStore()
	kept := &store.OutputStored{
		Output: host.WholeOutput{Records: []host.OutputRecord{{Stream: "stdout", Bytes: []byte("kept")}}},
	}
	memory.KeepOutput("1", kept)
	memory.KeepOutput("2", &store.OutputNotStored{Reason: "offline"})
	memory.KeepOutput("3", &store.OutputRemoved{At: contractEpoch})
	copied := memory.Copy()
	kept.Output.Records[0].Bytes[0] = 'x'
	for _, id := range []core.CommandID{"1", "2", "3"} {
		got, err := copied.CommandOutput(t.Context(), id)
		if err != nil {
			t.Fatal(err)
		}
		switch got := got.(type) {
		case *store.OutputStored:
			if string(got.Output.Records[0].Bytes) != "kept" {
				t.Fatal("output bytes changed after saving")
			}
			got.Output.Records[0].Bytes[0] = 'y'
			again, err := copied.CommandOutput(t.Context(), id)
			if err != nil || string(again.(*store.OutputStored).Output.Records[0].Bytes) != "kept" {
				t.Fatal("read exposed stored output bytes")
			}
		case *store.OutputNotStored:
			if got.Reason != "offline" {
				t.Fatal("missing output reason lost")
			}
		case *store.OutputRemoved:
			if got.At != contractEpoch {
				t.Fatal("removal time lost")
			}
		default:
			t.Fatalf("output %s missing", id)
		}
	}
	if got, err := copied.CommandOutput(t.Context(), "unknown"); err != nil || got != nil {
		t.Fatalf("unknown command: %v %v", got, err)
	}
}
