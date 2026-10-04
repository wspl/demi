package server_test

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
)

func TestTurnPatchesRebuildTranscript(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		script := providertest.NewScriptedRuntime(
			t,
			providertest.Events(
				&provider.ThinkingStart{},
				providertest.Thinking("Let me look."),
				&provider.ThinkingSignature{Signature: "anthropic:sig-1"},
				providertest.Text("Checking "),
				providertest.Text("the files."),
				providertest.ToolCall("call-1", "shell_exec", []byte(`{"script":"ls"}`)),
				providertest.Response(12, 8),
			),
			providertest.Events(providertest.Text("There are two files."), providertest.Response(30, 6)),
		)
		f := fixtureWith(
			t,
			script,
			storetest.NewMemoryTreeStore(),
			server.DefaultConfig(),
			func(deps *server.Deps[*toolstest.NoHost]) {
				deps.Clock = providertest.FixedClock("2026-09-24T12:00:00.000Z")
			},
		)
		c := f.client()
		c.Send(t.Context(), &conversationproto.OpenFrame{})
		handshake := c.Received()
		c.Send(t.Context(), send("m1", "List the files"))
		frames := untilIdle(t, c)
		patches := []conversationproto.ServerFrame{}
		for _, frame := range frames {
			if _, ok := frame.(*conversationproto.TranscriptPatchFrame); ok {
				patches = append(patches, frame)
			}
		}
		live := f.server.Tree(rootID()).Root().Session().Transcript()
		actual, err := contract.EncodeJSON([]struct {
			Name       string                          `json:"name"`
			Reset      conversationproto.ServerFrame   `json:"reset"`
			Patches    []conversationproto.ServerFrame `json:"patches"`
			Transcript []types.Block                   `json:"transcript"`
		}{{"a turn with signed thinking, streamed text and a tool call", handshake[1], patches, live.Blocks}})
		if err != nil {
			t.Fatal(err)
		}
		expected, err := os.ReadFile("testdata/transcript-patches.json")
		if err != nil {
			t.Fatal(err)
		}
		var compact bytes.Buffer
		if err := json.Compact(&compact, expected); err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(compact.Bytes(), actual) {
			if err := os.WriteFile(t.TempDir()+"/transcript-patches.actual.json", actual, 0o600); err != nil {
				t.Fatal(err)
			}
			equal(t, compact.String(), string(actual))
		}
		if record := f.store.Record(rootID()); record.Parent != nil || record.Closed != nil {
			t.Fatal(record)
		}

		c.Send(t.Context(), &conversationproto.CloseFrame{})
		closed := c.Received()
		if _, ok := closed[len(closed)-1].(*conversationproto.ClosedFrame); !ok {
			t.Fatal(closed)
		}
		if f.server.Tree(rootID()) != nil {
			t.Fatal("closed tree remains")
		}
		equal(t, 1, f.script.Closes())
		c.Send(t.Context(), &conversationproto.OpenFrame{})
		h := c.Received()
		reset := h[1].(*conversationproto.TranscriptResetFrame)
		equal(t, live.Blocks, reset.Blocks)
		if live.Version.Epoch == reset.Version.Epoch {
			t.Fatal("reopen reused epoch")
		}
	})
}

func TestProviderFailureIsPublishedOnceAndQueueContinues(t *testing.T) {
	for _, code := range []provider.ErrorCode{
		provider.AuthExpired,
		provider.AuthMissing,
		provider.ErrorCode("invalid_request_error"),
	} {
		t.Run(string(code), func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				gate := make(chan struct{})
				f := newFixture(
					t,
					held(gate, providertest.Text("partial"), providertest.Error("the vendor refused", &code)),
					said("second"),
				)
				c := f.opened()
				c.Send(t.Context(), send("m1", "first"))
				c.Send(t.Context(), send("m2", "second"))
				close(gate)
				synctest.Wait()
				equal(t, 2, len(f.script.Requests()))
				count := 0
				for _, frame := range c.Received() {
					if e, ok := frame.(*conversationproto.ErrorFrame); ok {
						count++
						equal(t, "the vendor refused", e.Message)
						equal(t, string(code), *e.Code)
						if e.Diagnostics == nil {
							t.Fatal("missing diagnostics")
						}
						equal(t, types.FailureSourceUnknown, e.Diagnostics.Source)
						equal(t, f.script.Requests()[0].RequestID, *e.Diagnostics.ClientRequestID)
					}
				}
				equal(t, 1, count)
				cp := f.store.Checkpoint(rootID())
				assertBlockTypes(t, cp.Transcript, "User", "Text", "Error", "User", "Text", "Response")
				equal(t, string(code), *cp.Transcript[2].(*types.ErrorBlock).Code)
				equal(t, types.SessionPhaseIdle, cp.State.Phase)
			})
		})
	}
}

func TestStopPublishesMarkerBeforeReplyAndRunsQueue(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, providertest.Pending(), said("second answer"))
		f.opened()
		paused, release := make(chan struct{}), make(chan struct{})
		observer := f.server.Tree(rootID()).Root().Session().Subscribe(func(event session.Event) {
			if change, ok := event.(*session.TranscriptChanged); ok {
				for _, patch := range change.Patches {
					if added, ok := patch.(*conversationproto.AddPatch); ok {
						if _, stopped := added.Value.(*types.AbortBlock); stopped {
							close(paused)
							<-release
						}
					}
				}
			}
		})
		defer observer.Release()
		c := f.opened()
		c.Send(t.Context(), send("m1", "first"))
		c.Send(t.Context(), send("m2", "second"))
		synctest.Wait()
		done := make(chan struct{})
		go func() {
			defer close(done)
			c.Send(t.Context(), &conversationproto.AbortFrame{})
		}()
		<-paused
		synctest.Wait()
		early := c.Received()
		close(release)
		<-done
		for _, frame := range early {
			if _, ok := frame.(*conversationproto.AbortResultFrame); ok {
				t.Fatal("abort reply preceded marker publication")
			}
		}
		frames, err := c.NextUntil(t.Context(), func(f conversationproto.ServerFrame) bool {
			_, ok := f.(*conversationproto.AbortResultFrame)
			return ok
		})
		if err != nil {
			t.Fatal(err)
		}
		marker := false
		for _, f := range frames {
			if p, ok := f.(*conversationproto.TranscriptPatchFrame); ok {
				for _, p := range p.Patches {
					if a, ok := p.(*conversationproto.AddPatch); ok {
						if _, ok := a.Value.(*types.AbortBlock); ok {
							marker = true
						}
					}
				}
			}
		}
		if !marker {
			t.Fatal("abort reply preceded marker")
		}
		reply := frames[len(frames)-1].(*conversationproto.AbortResultFrame)
		equal(t, conversationproto.AbortTargetActiveProviderStream, *reply.Result.Target)
		if !reply.Result.CanAbortAgain {
			t.Fatal(reply)
		}
		synctest.Wait()
		assertBlockTypes(
			t,
			f.server.Tree(rootID()).Root().Session().Transcript().Blocks,
			"User",
			"Abort",
			"User",
			"Text",
			"Response",
		)
		equal(t, "second answer", texts(f))
	})
}

func TestInterruptHoldsQueuedMessageUntilRelease(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, providertest.Pending(), said("second answer"))
		c := f.opened()
		c.Send(t.Context(), send("m1", "first"))
		c.Send(t.Context(), send("m2", "second"))
		synctest.Wait()
		tree := f.server.Tree(rootID())
		held, err := tree.Interrupt(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		synctest.Wait()
		equal(t, 1, len(f.script.Requests()))
		assertBlockTypes(t, tree.Root().Session().Transcript().Blocks, "User", "Abort")
		held.Release()
		synctest.Wait()
		equal(t, 2, len(f.script.Requests()))
		assertBlockTypes(t, tree.Root().Session().Transcript().Blocks, "User", "Abort", "User", "Text", "Response")
		equal(t, "second answer", texts(f))
	})
}

func TestCloseSavesInterruptionAndReopenRunsQueue(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, providertest.Pending(), said("later"))
		c := f.opened()
		c.Send(t.Context(), send("m1", "first"))
		c.Send(t.Context(), send("m2", "second"))
		synctest.Wait()
		c.Send(t.Context(), &conversationproto.CloseFrame{})
		closed := c.Received()
		if _, ok := closed[len(closed)-1].(*conversationproto.ClosedFrame); !ok {
			t.Fatal(closed)
		}
		cp := f.store.Checkpoint(rootID())
		equal(t, types.SessionPhaseRunning, cp.State.Phase)
		assertBlockTypes(t, cp.Transcript, "User", "Error")
		equal(t, 1, len(cp.State.Queue))
		equal(t, turnID("m2"), cp.State.Queue[0].ID)
		equal(t, 1, f.script.Closes())
		c.Send(t.Context(), &conversationproto.OpenFrame{})
		synctest.Wait()
		frames := c.Received()
		assertBlockTypes(t, frames[1].(*conversationproto.TranscriptResetFrame).Blocks, "User", "Error")
		assertBlockTypes(
			t,
			f.server.Tree(rootID()).Root().Session().Transcript().Blocks,
			"User",
			"Error",
			"User",
			"Text",
			"Response",
		)
		equal(t, "later", texts(f))
	})
}

func TestCrashRecordsInterruptionOnReopen(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, providertest.Pending())
		c := f.opened()
		c.Send(t.Context(), send("m1", "first"))
		synctest.Wait()
		time.Sleep(2 * time.Second)
		synctest.Wait()
		crashed := f.store.Copy()
		equal(t, types.SessionPhaseRunning, crashed.Checkpoint(rootID()).State.Phase)
		after := fixtureWith(t, providertest.NewScriptedRuntime(t), crashed, server.DefaultConfig())
		back := after.client()
		back.Send(t.Context(), &conversationproto.OpenFrame{})
		frames := back.Received()
		reset := frames[1].(*conversationproto.TranscriptResetFrame)
		if _, ok := frames[0].(*conversationproto.OpenedFrame); !ok {
			t.Fatal(frames)
		}
		assertBlockTypes(t, reset.Blocks, "User")
		patched := false
		for _, frame := range frames[5:] {
			if _, ok := frame.(*conversationproto.TranscriptPatchFrame); ok {
				patched = true
			}
		}
		if !patched {
			t.Fatal("interruption patch did not follow handshake")
		}
		cp := crashed.Checkpoint(rootID())
		assertBlockTypes(t, cp.Transcript, "User", "Error")
		equal(t, types.SessionPhaseIdle, cp.State.Phase)
	})
}

func switchModel(t *testing.T, f *fixture, model types.ModelSelection) {
	t.Helper()
	change, err := f.server.PrepareSwitch(t.Context(), rootID(), model)
	if err != nil {
		t.Fatal(err)
	}
	if change == nil {
		t.Fatal("no live tree")
	}
	if err := f.server.SwitchModel(t.Context(), rootID(), *change); err != nil {
		t.Fatal(err)
	}
}

func TestSwitchLandsAtNextRequestInsideTurn(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := make(chan struct{})
		f := newFixture(
			t,
			held(gate, providertest.ToolCall("call-1", "shell_exec", []byte(`{}`)), providertest.Response(1, 1)),
			said("one"),
			said("two"),
		)
		c := f.opened()
		c.Send(t.Context(), send("m1", "first"))
		synctest.Wait()
		model := storetest.ModelOf("stub", "model-b")
		model.Thinking = &types.EffortConfig{Effort: "high"}
		model.ServiceTierID = new("priority")
		switchModel(t, f, model)
		close(gate)
		untilIdle(t, c)
		c.Send(t.Context(), send("m2", "second"))
		untilIdle(t, c)
		requests := f.script.Requests()
		equal(t, 3, len(requests))
		equal(t, "test-model", requests[0].ModelID)
		equal(t, types.ThinkingConfig(nil), requests[0].Thinking)
		equal(t, (*string)(nil), requests[0].ServiceTierID)
		for _, r := range requests[1:] {
			equal(t, "model-b", r.ModelID)
			equal(t, model.Thinking, r.Thinking)
			equal(t, model.ServiceTierID, r.ServiceTierID)
		}
		equal(t, 1, len(f.resolver.Calls()))
		blocks := f.server.Tree(rootID()).Root().Session().Transcript().Blocks
		assertBlockTypes(t, blocks, "User", "ToolCall", "Response", "Text", "Response", "User", "Text", "Response")
		equal(t, types.ToolCallStatus("error"), blocks[1].(*types.ToolCallBlock).Status)
		for i, b := range blocks {
			want := "model-b"
			if i < 3 {
				want = "test-model"
			}
			equal(t, want, b.Model().Model.ID)
		}
		equal(t, "model-b", f.store.Checkpoint(rootID()).State.Model.Model.ID)
	})
}

func TestSwitchProviderOwnsAndClosesRuntimes(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, said("from stub"))
		replaced := providertest.NewScriptedRuntime(t)
		other := providertest.NewScriptedRuntime(t, said("from other"))
		f.resolver.Provide("replaced", replaced)
		f.resolver.Provide("other", other)
		c := f.opened()
		c.Send(t.Context(), send("m1", "first"))
		untilIdle(t, c)
		for _, m := range []types.ModelSelection{
			storetest.ModelOf("replaced", "replaced-model"),
			storetest.ModelOf("other", "other-model"),
			storetest.ModelOf("other", "other-model-2"),
		} {
			switchModel(t, f, m)
		}
		c.Send(t.Context(), send("m2", "second"))
		untilIdle(t, c)
		calls := f.resolver.Calls()
		equal(t, 3, len(calls))
		for i, call := range calls {
			equal(t, rootID(), call.Root)
			equal(t, []string{"stub", "replaced", "other"}[i], call.Provider)
		}
		equal(t, 1, len(f.script.Requests()))
		equal(t, 1, f.script.Closes())
		equal(t, 1, replaced.Closes())
		equal(t, 0, len(replaced.Requests()))
		equal(t, 1, len(other.Requests()))
		equal(t, "other-model-2", other.Requests()[0].ModelID)
		equal(t, "other", f.store.Checkpoint(rootID()).State.Model.ProviderID)
		equal(t, "other-model-2", f.store.Checkpoint(rootID()).State.Model.Model.ID)
		equal(t, 0, other.Closes())
	})
}

func TestQueueCanBeReorderedAndEmptied(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		first, last := make(chan struct{}), make(chan struct{})
		f := newFixture(
			t,
			held(first, providertest.Text("one"), providertest.Response(1, 1)),
			said("three"),
			held(last, providertest.Text("two"), providertest.Response(1, 1)),
		)
		c := f.opened()
		c.Send(t.Context(), send("m1", "one"))
		synctest.Wait()
		for _, id := range []string{"m2", "m3", "m4", "m5"} {
			c.Send(t.Context(), send(id, id))
		}
		c.Send(t.Context(), &conversationproto.SendQueuedMessageFrame{MessageID: turnID("m3")})
		c.Send(t.Context(), &conversationproto.DequeueMessageFrame{MessageID: turnID("m4")})
		queues := [][]string{}
		for _, frame := range c.Received() {
			if q, ok := frame.(*conversationproto.QueueFrame); ok {
				ids := []string{}
				for _, message := range q.Queue {
					ids = append(ids, string(message.ID))
				}
				queues = append(queues, ids)
			}
		}
		equal(
			t,
			[][]string{
				{"m2"},
				{"m2", "m3"},
				{"m2", "m3", "m4"},
				{"m2", "m3", "m4", "m5"},
				{"m3", "m2", "m4", "m5"},
				{"m3", "m2", "m5"},
			},
			queues,
		)
		queue := f.server.Tree(rootID()).Root().Session().QueuedMessages()
		equal(t, 3, len(queue))
		equal(t, turnID("m3"), queue[0].ID)
		close(first)
		synctest.Wait()
		equal(t, 3, len(f.script.Requests()))
		c.Send(t.Context(), &conversationproto.ClearMessageQueueFrame{})
		close(last)
		synctest.Wait()
		users := []types.TurnID{}
		for _, b := range f.server.Tree(rootID()).Root().Session().Transcript().Blocks {
			if u, ok := b.(*types.UserBlock); ok {
				users = append(users, u.TurnID)
			}
		}
		equal(t, []types.TurnID{turnID("m1"), turnID("m3"), turnID("m2")}, users)
		equal(t, 0, len(f.server.Tree(rootID()).Root().Session().QueuedMessages()))
	})
}

// assertBlockTypes checks the complete sequence of transcript block variants.
func assertBlockTypes(t *testing.T, blocks []types.Block, names ...string) {
	t.Helper()
	equal(t, len(names), len(blocks))
	for i, name := range names {
		equal(t, "*types."+name+"Block", fmt.Sprintf("%T", blocks[i]))
	}
}
