package server_test

import (
	"slices"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
)

func storedChild(id string, number uint64, parent types.NodeID, profile *string) store.NodeRecord {
	return store.NodeRecord{
		ID:                types.NodeID(id),
		Number:            number,
		Parent:            &parent,
		Description:       id,
		Profile:           profile,
		Round:             1,
		StartedAt:         types.Timestamp("1970-01-01T00:00:00.000Z"),
		CanSpawnSubagents: true,
	}
}

func checkpoint(queue []types.QueuedMessage, blocks []types.Block) store.CheckpointUpdate {
	rows := []store.ChangedBlock{}
	for i, b := range blocks {
		rows = append(rows, store.ChangedBlock{Index: i, Block: b})
	}
	return store.CheckpointUpdate{
		State: store.CheckpointState{
			Phase:       types.SessionPhaseIdle,
			Queue:       append([]types.QueuedMessage{}, queue...),
			AgentInputs: []store.PendingAgentInput{},
			Wakeups:     []store.ScheduledWakeup{},
			CWD:         "/workspace",
			Model:       storetest.TestModel(),
			Edits:       []store.EditReceipt{},
		},
		CommandState:  new(store.InitialCommandState()),
		BlockCount:    len(blocks),
		ChangedBlocks: rows,
	}
}

func createStored(
	t *testing.T,
	memory *storetest.MemoryTreeStore,
	record store.NodeRecord,
	queue []types.QueuedMessage,
	blocks []types.Block,
) {
	t.Helper()
	if err := memory.CreateNode(t.Context(), record, checkpoint(queue, blocks)); err != nil {
		t.Fatal(err)
	}
}

func closeStored(t *testing.T, memory *storetest.MemoryTreeStore, id types.NodeID, phase store.ClosePhase) {
	t.Helper()
	if err := memory.CloseNode(
		t.Context(),
		id,
		store.NodeClose{Phase: phase, At: types.Timestamp("1970-01-01T00:00:00.000Z")},
	); err != nil {
		t.Fatal(err)
	}
}

func TestRestoreLostBriefQuietChildAndMissedCompletion(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		memory := storetest.NewMemoryTreeStore()
		at := types.Timestamp("1970-01-01T00:00:00.000Z")
		createStored(t, memory, store.RootRecord(rootID(), at), nil, nil)
		createStored(
			t,
			memory,
			storedChild("lost", 1, rootID(), nil),
			[]types.QueuedMessage{{ID: turnID("brief"), Content: storetest.Text("task lost")}},
			nil,
		)
		createStored(
			t,
			memory,
			storedChild("quiet", 2, rootID(), nil),
			nil,
			[]types.Block{
				&types.UserBlock{
					BlockID:   blockID("q1"),
					TurnID:    turnID("q1"),
					Timestamp: at,
					Selection: storetest.TestModel(),
					Content:   storetest.Text("task quiet"),
				},
				&types.TextBlock{
					BlockID:   blockID("q2"),
					Timestamp: at,
					Selection: storetest.TestModel(),
					Text:      "quiet result",
					Forkable:  true,
				},
			},
		)
		createStored(t, memory, storedChild("closed", 3, rootID(), nil), nil, nil)
		closeStored(t, memory, "closed", &store.Completed{Result: "closed result"})
		createStored(t, memory, storedChild("orphan", 4, rootID(), new("retired")), nil, nil)
		createStored(t, memory, storedChild("archived", 5, rootID(), new("retired")), nil, nil)
		closeStored(t, memory, "archived", &store.Aborted{})
		if err := memory.MarkDelivered(t.Context(), "archived", 1); err != nil {
			t.Fatal(err)
		}
		createStored(t, memory, storedChild("orphan-child", 6, "orphan", nil), nil, nil)
		rootScript := providertest.NewScriptedRuntime(t, said("noted"), said("noted"), said("noted"))
		lost := providertest.NewScriptedRuntime(t, said("lost result"))
		f := fixtureWith(t, rootScript, memory, server.DefaultConfig())
		f.resolver.ProvideRuntime("stub", &nodeScripts{root: rootScript, children: []childScript{{"task lost", lost}}})
		c := f.client()
		c.Send(t.Context(), &conversationproto.OpenFrame{})
		synctest.Wait()
		senders := []types.NodeID{}
		for _, receipt := range agentReceipts(f, rootID()) {
			senders = append(senders, receipt.Sender.ID)
		}
		slices.Sort(senders)
		equal(t, []types.NodeID{"closed", "lost", "quiet"}, senders)
		for i, id := range []types.NodeID{"lost", "quiet", "closed"} {
			record := memory.Record(id)
			if record.Closed == nil || !record.Delivered {
				t.Fatal("restore did not close and deliver", id, record)
			}
			equal(
				t,
				store.ClosePhase(&store.Completed{Result: []string{"lost result", "quiet result", "closed result"}[i]}),
				record.Closed.Phase,
			)
		}
		if memory.Record("orphan") != nil || memory.Record("orphan-child") != nil {
			t.Fatal("missing profile left subtree")
		}
		pending := false
		for _, frame := range c.Received() {
			if _, ok := frame.(*conversationproto.PendingSteersFrame); ok {
				pending = true
			}
		}
		if !pending {
			t.Fatal("no pending steers handshake")
		}
		run := agent(t, f, rootID(), "resume", `{"id":5,"message":"again"}`)
		equal(t, uint8(1), run.code)
		equal(
			t,
			"demi agent resume: unknown profile \"retired\" (available: none; omit --profile to inherit the parent)\n",
			run.stderr,
		)
		equal(t, store.ClosePhase(&store.Aborted{}), memory.Record("archived").Closed.Phase)
	})
}

func TestUndeliveredCompletionRefusesEditUntilLaterSave(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, said("answer A"), said("answer B"), said("answer A2"))
		c := f.opened()
		c.Send(t.Context(), send("m1", "A"))
		untilIdle(t, c)
		c.Send(t.Context(), &conversationproto.CloseFrame{})
		c.Received()
		createStored(t, f.store, storedChild("child", 1, rootID(), nil), nil, nil)
		closeStored(t, f.store, "child", &store.Completed{Result: "notes say 42"})
		f.store.FailSaves(int(^uint(0) >> 1))
		c.Send(t.Context(), &conversationproto.OpenFrame{})
		synctest.Wait()
		f.store.FailSaves(0)
		record := f.store.Record("child")
		if record.Delivered {
			t.Fatal("failed save delivered completion")
		}
		target := userBlock(t, f, "m1")
		c.Send(t.Context(), edit("op1", target, f.server.Tree(rootID()).Root().Session().Transcript().Version, "A2"))
		synctest.Wait()
		rejectedEdit(
			t,
			editOutcome(t, c.Received()),
			"Cannot edit while children or completion notifications are pending",
		)
		equal(t, record, f.store.Record("child"))
		c.Send(t.Context(), send("m2", "B"))
		untilIdle(t, c)
		if !f.store.Record("child").Delivered {
			t.Fatal("later save did not deliver")
		}
		c.Send(t.Context(), edit("op2", target, f.server.Tree(rootID()).Root().Session().Transcript().Version, "A2"))
		frames := untilIdle(t, c)
		if _, ok := editOutcome(t, frames).(*conversationproto.AcceptedEdit); !ok {
			t.Fatal(frames)
		}
		equal(t, 0, f.script.Remaining())
	})
}
