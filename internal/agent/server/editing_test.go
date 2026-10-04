package server_test

import (
	"context"
	"errors"
	"fmt"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/agent/server/servertest"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func userBlock(t *testing.T, f *fixture, turn string) core.BlockID {
	t.Helper()
	for _, b := range f.server.Tree(rootID()).Root().Session().Transcript().Blocks {
		if u, ok := b.(*core.UserBlock); ok && u.TurnID == turnID(turn) {
			return u.ID()
		}
	}
	t.Fatal("no user block", turn)
	return ""
}

func edit(
	operation string,
	target core.BlockID,
	version framewire.TranscriptVersion,
	text string,
) *framewire.EditAndSendFrame {
	id, err := core.ParseOperationID(operation)
	if err != nil {
		panic(err)
	}
	return &framewire.EditAndSendFrame{
		Request: framewire.EditRequest{
			OperationID:   id,
			TargetBlockID: target,
			Version:       version,
			Content:       servertest.ClientText(text),
		},
	}
}

func editOutcome(t *testing.T, frames []framewire.ServerFrame) framewire.EditOutcome {
	t.Helper()
	for _, frame := range frames {
		if frame, ok := frame.(*framewire.EditResultFrame); ok {
			return frame.Outcome
		}
	}
	t.Fatal("no edit result", frames)
	return nil
}

func rejectedEdit(t *testing.T, outcome framewire.EditOutcome, reason string) {
	t.Helper()
	r, ok := outcome.(*framewire.RejectedEdit)
	if !ok {
		t.Fatal(outcome)
	}
	equal(t, reason, r.Reason)
}

func TestEditReplacesOnceOnFreshRuntime(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, said("answer A"), said("answer B"), said("answer C"), said("answer B2"))
		c := f.opened()
		for i, v := range []struct{ id, text string }{{"m1", "A"}, {"m2", "B"}, {"m3", "C"}} {
			if i > 0 {
				writeStorage(t, f, fmt.Sprint(i), nil)
			}
			c.Send(t.Context(), send(v.id, v.text))
			untilIdle(t, c)
		}
		before := f.server.Tree(rootID()).Root().Session().Transcript()
		target := userBlock(t, f, "m2")
		c.Send(t.Context(), edit("op0", before.Blocks[1].ID(), before.Version, "x"))
		rejectedEdit(t, editOutcome(t, c.Received()), "The edit target must be a user message")
		synctest.Wait()
		c.Received()
		request := edit("op1", target, before.Version, "B2")
		c.Send(t.Context(), request)
		frames := untilIdle(t, c)
		accepted, ok := editOutcome(t, frames).(*framewire.AcceptedEdit)
		if !ok {
			t.Fatal(frames)
		}
		rewrites := 0
		for i, frame := range frames {
			if p, ok := frame.(*framewire.TranscriptPatchFrame); ok {
				for _, p := range p.Patches {
					if r, ok := p.(*framewire.ReplacePatch); ok {
						rewrites++
						equal(t, before.Blocks[:3], r.Value[:3])
						equal(t, 4, len(r.Value))
						replacement := r.Value[3].(*core.UserBlock)
						equal(t, accepted.TurnID, replacement.TurnID)
						equal(t, storetest.Text("B2"), replacement.Content)
						equal(t, r.Value, f.server.Tree(rootID()).Root().Session().Transcript().Blocks[:4])
						if _, ok := frames[i+1].(*framewire.EditResultFrame); !ok {
							t.Fatalf("acceptance did not follow rewrite: %T", frames[i+1])
						}
					}
				}
			}
		}
		equal(t, 1, rewrites)
		live := f.server.Tree(rootID()).Root().Session().Transcript()
		assertBlockTypes(t, live.Blocks, "User", "Text", "Response", "User", "Text", "Response")
		equal(
			t,
			[]provider.InferenceItem{
				&provider.UserMessage{Content: storetest.SentText("A")},
				&provider.AssistantText{ModelID: "test-model", Text: "answer A"},
				&provider.UserMessage{Content: storetest.SentText("B2")},
			},
			f.script.Requests()[3].Items,
		)
		equal(t, 1, f.script.Closes())
		equal(t, live.Blocks, f.store.Checkpoint(rootID()).Transcript)
		equal(t, 1, len(f.store.Checkpoint(rootID()).State.Edits))
		equal(t, accepted.TurnID, f.store.Checkpoint(rootID()).State.Edits[0].TurnID)
		reply, err := f.server.CommandStorage(
			t.Context(),
			rootID(),
			f.server.Node(rootID(), rootID()).JobCaller(),
			&host.StorageRead{Key: "todo"},
		)
		if err != nil {
			t.Fatal(err)
		}
		equal(t, host.StorageReply(&host.StorageValue{Value: []byte("1"), Revision: 1}), reply)
		c.Send(t.Context(), request)
		equal(t, framewire.EditOutcome(accepted), editOutcome(t, c.Received()))
		c.Send(t.Context(), edit("op1", target, live.Version, "B3"))
		rejectedEdit(t, editOutcome(t, c.Received()), "The edit operation ID was used for a different request")
		c.Send(t.Context(), edit("op2", target, before.Version, "B3"))
		rejectedEdit(t, editOutcome(t, c.Received()), "The conversation changed; reopen the message to edit it")
		c.Send(t.Context(), &framewire.CloseFrame{})
		c.Received()
		c.Send(t.Context(), &framewire.OpenFrame{})
		c.Received()
		c.Send(t.Context(), edit("op3", userBlock(t, f, string(accepted.TurnID)), live.Version, "B4"))
		rejectedEdit(t, editOutcome(t, c.Received()), "The conversation changed; reopen the message to edit it")
		c.Send(t.Context(), request)
		equal(t, framewire.EditOutcome(accepted), editOutcome(t, c.Received()))
		equal(t, 0, f.script.Remaining())
	})
}

func TestEditVisibleOnlyAfterCommit(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, said("answer A"), said("answer A2"))
		c, other := f.opened(), f.opened()
		c.Send(t.Context(), send("m1", "A"))
		untilIdle(t, c)
		untilIdle(t, other)
		r := edit("op1", userBlock(t, f, "m1"), f.server.Tree(rootID()).Root().Session().Transcript().Version, "A2")
		gate := f.store.HoldSaves()
		defer gate.Release()
		done := make(chan struct{})
		go func() {
			defer close(done)
			c.Send(t.Context(), r)
		}()
		if err := gate.Wait(t.Context(), 1); err != nil {
			t.Fatal(err)
		}
		for _, frame := range c.Received() {
			if _, ok := frame.(*framewire.TranscriptResetFrame); ok {
				t.Fatal("edit reset before commit")
			}
			if _, ok := frame.(*framewire.EditResultFrame); ok {
				t.Fatal("edit result before commit")
			}
			if _, ok := frame.(*framewire.TranscriptPatchFrame); ok {
				t.Fatal("edit patch before commit")
			}
		}
		joined := make(chan struct{})
		go func() {
			defer close(joined)
			other.Send(t.Context(), r)
		}()
		synctest.Wait()
		gate.Release()
		<-joined
		<-done
		for _, client := range []*servertest.TestClient[*toolstest.NoHost]{c, other} {
			frames := untilIdle(t, client)
			results := 0
			for i, frame := range frames {
				if _, ok := frame.(*framewire.EditResultFrame); ok {
					results++
					if i == 0 {
						t.Fatal("acceptance before rewrite")
					}
					patch, ok := frames[i-1].(*framewire.TranscriptPatchFrame)
					if !ok {
						t.Fatalf("acceptance follows %T", frames[i-1])
					}
					if _, ok := patch.Patches[0].(*framewire.ReplacePatch); !ok {
						t.Fatal("acceptance did not follow rewrite")
					}
				}
			}
			equal(t, 1, results)
			if _, ok := editOutcome(t, frames).(*framewire.AcceptedEdit); !ok {
				t.Fatal(frames)
			}
		}
	})
}

func TestSwitchDuringPreparedEditLandsAfterFirstRequest(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(
			t,
			said("answer A"),
			providertest.Events(
				providertest.ToolCall("call-1", "shell_exec", []byte(`{}`)),
				providertest.Response(1, 1),
			),
			said("answer A2"),
		)
		c := f.opened()
		c.Send(t.Context(), send("m1", "A"))
		untilIdle(t, c)
		r := edit("op1", userBlock(t, f, "m1"), f.server.Tree(rootID()).Root().Session().Transcript().Version, "A2")
		gate := f.store.HoldSaves()
		defer gate.Release()
		done := make(chan struct{})
		go func() {
			defer close(done)
			c.Send(t.Context(), r)
		}()
		if err := gate.Wait(t.Context(), 1); err != nil {
			t.Fatal(err)
		}
		switchModel(t, f, storetest.ModelOf("stub", "model-b"))
		gate.Release()
		<-done
		if _, ok := editOutcome(t, untilIdle(t, c)).(*framewire.AcceptedEdit); !ok {
			t.Fatal("edit refused")
		}
		models := []string{}
		for _, r := range f.script.Requests() {
			models = append(models, r.ModelID)
		}
		equal(t, []string{"test-model", "test-model", "model-b"}, models)
	})
}

func TestBusyEditAndFailedSaveChangeNothing(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := make(chan struct{})
		f := newFixture(t, said("A"), held(gate, providertest.Text("B"), providertest.Response(1, 1)), said("A2"))
		c := f.opened()
		c.Send(t.Context(), send("m1", "A"))
		untilIdle(t, c)
		target := userBlock(t, f, "m1")
		c.Send(t.Context(), send("m2", "B"))
		synctest.Wait()
		c.Send(t.Context(), edit("op1", target, f.server.Tree(rootID()).Root().Session().Transcript().Version, "A2"))
		rejectedEdit(t, editOutcome(t, c.Received()), "Message editing requires a settled session with no pending work")
		close(gate)
		untilIdle(t, c)
		before := f.server.Tree(rootID()).Root().Session().Transcript()
		caller := f.server.Node(rootID(), rootID()).JobCaller()
		r := edit("op2", target, before.Version, "A2")
		f.store.FailSaves(1)
		c.Send(t.Context(), r)
		rejectedEdit(t, editOutcome(t, c.Received()), "the database refused the save")
		equal(t, before, f.server.Tree(rootID()).Root().Session().Transcript())
		if _, err := f.server.CommandStorage(
			t.Context(),
			rootID(),
			caller,
			&host.StorageRead{Key: "todo"},
		); err != nil {
			t.Fatal(err)
		}
		equal(t, before.Blocks, f.store.Checkpoint(rootID()).Transcript)
		equal(t, 0, len(f.store.Checkpoint(rootID()).State.Edits))
		// Consume the failed edit's idle publication before observing the retry.
		synctest.Wait()
		c.Received()
		c.Send(t.Context(), r)
		if _, ok := editOutcome(t, untilIdle(t, c)).(*framewire.AcceptedEdit); !ok {
			t.Fatal("retry refused")
		}
		blocks := f.server.Tree(rootID()).Root().Session().Transcript().Blocks
		assertBlockTypes(t, blocks, "User", "Text", "Response")
	})
}

func TestCommandStorageGenerationAndCancellation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t)
		c := f.opened()
		caller := f.server.Node(rootID(), rootID()).JobCaller()
		write := &host.StorageWriteIf{Key: "todos.json", Value: []byte(`["T1"]`), Expected: new(host.Revision(0))}
		reply, err := f.server.CommandStorage(t.Context(), rootID(), caller, write)
		if err != nil {
			t.Fatal(err)
		}
		equal(t, host.StorageReply(&host.StorageCommitted{Revision: 1}), reply)
		reply, err = f.server.CommandStorage(t.Context(), rootID(), caller, &host.StorageRead{Key: "todos.json"})
		if err != nil {
			t.Fatal(err)
		}
		equal(t, host.StorageReply(&host.StorageValue{Value: []byte(`["T1"]`), Revision: 1}), reply)
		stale := caller
		stale.Generation++
		_, err = f.server.CommandStorage(t.Context(), rootID(), stale, write)
		assertStorageError(t, err, "the command storage handle is no longer current")
		ctx, cancel := context.WithCancel(t.Context())
		cancel()
		// A canceled call ends with its context's error and commits nothing;
		// the revision check at the end proves the write did not land.
		_, err = f.server.CommandStorage(ctx, rootID(), caller, write)
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("expected the call's cancellation, got %v", err)
		}
		c.Send(t.Context(), &framewire.CloseFrame{})
		_, err = f.server.CommandStorage(t.Context(), rootID(), caller, write)
		assertStorageError(t, err, "the command storage handle is no longer current")
		equal(t, uint64(1), f.store.Checkpoint(rootID()).CommandState.Revision)
	})
}

func TestEditAndChildLifecycleRefuseEachOther(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		reading := make(chan struct{})
		reader := providertest.NewScriptedRuntime(
			t,
			held(reading, providertest.Text("notes say 42"), providertest.Response(1, 1)),
		)
		counter := providertest.NewScriptedRuntime(t, said("7 files"))
		f := modelFixture(
			t,
			[]providertest.Turn{said("answer A"), said("noted"), said("answer A2"), said("noted again")},
			childScriptEntry("Read notes.md", reader),
			childScriptEntry("Count the files", counter),
		)
		c := f.opened()
		c.Send(t.Context(), send("m1", "A"))
		untilIdle(t, c)
		target := userBlock(t, f, "m1")
		child := spawn(t, f, rootID(), `{"prompt":"Read notes.md"}`)
		live := f.store.Record(child)
		c.Send(t.Context(), edit("op1", target, f.server.Tree(rootID()).Root().Session().Transcript().Version, "A2"))
		synctest.Wait()
		rejectedEdit(
			t,
			editOutcome(t, c.Received()),
			"Cannot edit while children or completion notifications are pending",
		)
		equal(t, live, f.store.Record(child))
		close(reading)
		synctest.Wait()
		c.Received()
		delivered := f.store.Record(child)
		if delivered.Closed == nil || !delivered.Delivered {
			t.Fatal("child not delivered")
		}
		r := edit("op2", target, f.server.Tree(rootID()).Root().Session().Transcript().Version, "A2")
		gate := f.store.HoldSaves()
		defer gate.Release()
		done := make(chan struct{})
		go func() {
			defer close(done)
			c.Send(t.Context(), r)
		}()
		if err := gate.Wait(t.Context(), 1); err != nil {
			t.Fatal(err)
		}
		run := agent(t, f, rootID(), "spawn", `{"prompt":"Count the files"}`)
		equal(t, uint8(1), run.code)
		equal(t, "demi agent spawn: Cannot change children while a transcript edit is being prepared\n", run.stderr)
		gate.Release()
		<-done
		synctest.Wait()
		accepted := editOutcome(t, c.Received()).(*framewire.AcceptedEdit)
		equal(t, delivered, f.store.Record(child))
		target = userBlock(t, f, string(accepted.TurnID))
		version := f.server.Tree(rootID()).Root().Session().Transcript().Version
		gate = f.store.HoldSaves()
		defer gate.Release()
		started := make(chan commandRun, 1)
		go func() { started <- agent(t, f, rootID(), "spawn", `{"prompt":"Count the files"}`) }()
		if err := gate.Wait(t.Context(), 1); err != nil {
			t.Fatal(err)
		}
		c.Send(t.Context(), edit("op3", target, version, "A3"))
		rejectedEdit(t, editOutcome(t, c.Received()), "Cannot edit while a child lifecycle operation is in progress")
		gate.Release()
		equal(t, uint8(0), (<-started).code)
		synctest.Wait()
		equal(t, 0, f.script.Remaining())
		equal(t, 0, reader.Remaining())
		equal(t, 0, counter.Remaining())
	})
}

// assertStorageError checks the command storage refusal at its caller boundary.
func assertStorageError(t *testing.T, err error, message string) {
	t.Helper()
	var port *host.PortError
	if !errors.As(err, &port) {
		t.Fatalf("expected storage refusal, got %v", err)
	}
	equal(t, host.StorageRefused, port.Kind)
	equal(t, message, port.Message)
}
