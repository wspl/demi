package server_test

import (
	"bytes"
	"encoding/json"
	"os"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/tools/toolstest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestTurnPatchesRebuildTranscript(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {

		script := providertest.NewScriptedRuntime(t, providertest.Events(&provider.ThinkingStart{}, providertest.Thinking("Let me look."), &provider.ThinkingSignature{Signature: "anthropic:sig-1"}, providertest.Text("Checking "), providertest.Text("the files."), providertest.ToolCall("call-1", "shell_exec", []byte(`{"script":"ls"}`)), providertest.Response(12, 8)), providertest.Events(providertest.Text("There are two files."), providertest.Response(30, 6)))
		f := fixtureWith(t, script, storetest.NewMemoryTreeStore(), server.DefaultConfig(), func(deps *server.Deps[*toolstest.NoHost]) {
			deps.Clock = providertest.FixedClock("2026-09-24T12:00:00.000Z")
		})
		c := f.client()
		c.Send(t.Context(), &framewire.OpenFrame{})
		handshake := c.Received()
		c.Send(t.Context(), send("m1", "List the files"))
		frames := untilIdle(t, c)
		patches := []framewire.ServerFrame{}
		for _, frame := range frames {
			if _, ok := frame.(*framewire.TranscriptPatchFrame); ok {
				patches = append(patches, frame)
			}
		}
		live := f.server.Tree(rootID()).Root().Session().Transcript()
		actual, err := contract.EncodeJSON([]struct {
			Name       string                  `json:"name"`
			Reset      framewire.ServerFrame   `json:"reset"`
			Patches    []framewire.ServerFrame `json:"patches"`
			Transcript []core.Block            `json:"transcript"`
		}{{"a turn with signed thinking, streamed text and a tool call", handshake[1], patches, live.Blocks}})
		if err != nil {
			t.Fatal(err)
		}
		expected, err := os.ReadFile("testdata/transcript-patches.json")
		if err != nil {
			t.Fatal(err)
		}
		// The Go contract decoder names the same missing field in its standard
		// diagnostic format; every other fixture byte stays the Rust reference.
		expected = bytes.ReplaceAll(expected, []byte("missing field `timeoutMs`"), []byte("timeoutMs: required field is absent"))
		var compact bytes.Buffer
		if err := json.Compact(&compact, expected); err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(compact.Bytes(), actual) {
			if err := os.WriteFile(t.TempDir()+"/transcript-patches.actual.json", actual, 0600); err != nil {
				t.Fatal(err)
			}
			equal(t, compact.String(), string(actual))
		}
		if record := f.store.Record(rootID()); record.Parent != nil || record.Closed != nil {
			t.Fatal(record)
		}

		c.Send(t.Context(), &framewire.CloseFrame{})
		c.Received()
		c.Send(t.Context(), &framewire.OpenFrame{})
		h := c.Received()
		reset := h[1].(*framewire.TranscriptResetFrame)
		equal(t, live.Blocks, reset.Blocks)
		if live.Version.Epoch == reset.Version.Epoch {
			t.Fatal("reopen reused epoch")
		}
	})
}
func TestProviderFailureIsPublishedOnceAndQueueContinues(t *testing.T) {
	for _, code := range []provider.ErrorCode{provider.AuthExpired, provider.AuthMissing, provider.ErrorCode("invalid_request_error")} {
		t.Run(string(code), func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				gate := make(chan struct{})
				f := newFixture(t, held(gate, providertest.Text("partial"), providertest.Error("the vendor refused", &code)), said("second"))
				c := f.opened()
				c.Send(t.Context(), send("m1", "first"))
				c.Send(t.Context(), send("m2", "second"))
				close(gate)
				synctest.Wait()
				equal(t, 2, len(f.script.Requests()))
				count := 0
				for _, frame := range c.Received() {
					if e, ok := frame.(*framewire.ErrorFrame); ok {
						count++
						equal(t, "the vendor refused", e.Message)
						equal(t, string(code), *e.Code)
						if e.Diagnostics == nil {
							t.Fatal("missing diagnostics")
						}
						equal(t, f.script.Requests()[0].RequestID, *e.Diagnostics.ClientRequestID)
					}
				}
				equal(t, 1, count)
				cp := f.store.Checkpoint(rootID())
				equal(t, 6, len(cp.Transcript))
				equal(t, core.SessionPhaseIdle, cp.State.Phase)
			})
		})
	}
}
func TestStopPublishesMarkerBeforeReplyAndRunsQueue(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := newFixture(t, providertest.Pending(), said("second answer"))
		c := f.opened()
		c.Send(t.Context(), send("m1", "first"))
		c.Send(t.Context(), send("m2", "second"))
		synctest.Wait()
		c.Send(t.Context(), &framewire.AbortFrame{})
		frames, err := c.NextUntil(t.Context(), func(f framewire.ServerFrame) bool {
			_, ok := f.(*framewire.AbortResultFrame)
			return ok
		})
		if err != nil {
			t.Fatal(err)
		}
		marker := false
		for _, f := range frames {
			if p, ok := f.(*framewire.TranscriptPatchFrame); ok {
				for _, p := range p.Patches {
					if a, ok := p.(*framewire.AddPatch); ok {
						if _, ok := a.Value.(*core.AbortBlock); ok {
							marker = true
						}
					}
				}
			}
		}
		if !marker {
			t.Fatal("abort reply preceded marker")
		}
		reply := frames[len(frames)-1].(*framewire.AbortResultFrame)
		equal(t, framewire.AbortTargetActiveProviderStream, *reply.Result.Target)
		if !reply.Result.CanAbortAgain {
			t.Fatal(reply)
		}
		synctest.Wait()
		equal(t, 5, len(f.server.Tree(rootID()).Root().Session().Transcript().Blocks))
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
		equal(t, 2, len(tree.Root().Session().Transcript().Blocks))
		held.Release()
		synctest.Wait()
		equal(t, 2, len(f.script.Requests()))
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
		c.Send(t.Context(), &framewire.CloseFrame{})
		c.Received()
		cp := f.store.Checkpoint(rootID())
		equal(t, core.SessionPhaseRunning, cp.State.Phase)
		equal(t, 2, len(cp.Transcript))
		equal(t, 1, len(cp.State.Queue))
		equal(t, turnID("m2"), cp.State.Queue[0].ID)
		equal(t, 1, f.script.Closes())
		c.Send(t.Context(), &framewire.OpenFrame{})
		synctest.Wait()
		equal(t, 5, len(f.server.Tree(rootID()).Root().Session().Transcript().Blocks))
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
		equal(t, core.SessionPhaseRunning, crashed.Checkpoint(rootID()).State.Phase)
		after := fixtureWith(t, providertest.NewScriptedRuntime(t), crashed, server.DefaultConfig())
		back := after.client()
		back.Send(t.Context(), &framewire.OpenFrame{})
		frames := back.Received()
		reset := frames[1].(*framewire.TranscriptResetFrame)
		equal(t, 1, len(reset.Blocks))
		cp := crashed.Checkpoint(rootID())
		equal(t, 2, len(cp.Transcript))
		equal(t, core.SessionPhaseIdle, cp.State.Phase)
	})
}
func switchModel(t *testing.T, f *fixture, model core.ModelSelection) {
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
		f := newFixture(t, held(gate, providertest.ToolCall("call-1", "shell_exec", []byte(`{}`)), providertest.Response(1, 1)), said("one"), said("two"))
		c := f.opened()
		c.Send(t.Context(), send("m1", "first"))
		synctest.Wait()
		model := storetest.ModelOf("stub", "model-b")
		model.Thinking = &core.EffortConfig{Effort: "high"}
		model.ServiceTierID = new("priority")
		switchModel(t, f, model)
		close(gate)
		untilIdle(t, c)
		c.Send(t.Context(), send("m2", "second"))
		untilIdle(t, c)
		requests := f.script.Requests()
		equal(t, 3, len(requests))
		equal(t, "test-model", requests[0].ModelID)
		for _, r := range requests[1:] {
			equal(t, "model-b", r.ModelID)
			equal(t, model.Thinking, r.Thinking)
			equal(t, model.ServiceTierID, r.ServiceTierID)
		}
		equal(t, 1, len(f.resolver.Calls()))
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
		for _, m := range []core.ModelSelection{storetest.ModelOf("replaced", "replaced-model"), storetest.ModelOf("other", "other-model"), storetest.ModelOf("other", "other-model-2")} {
			switchModel(t, f, m)
		}
		c.Send(t.Context(), send("m2", "second"))
		untilIdle(t, c)
		equal(t, 3, len(f.resolver.Calls()))
		equal(t, 1, f.script.Closes())
		equal(t, 1, replaced.Closes())
		equal(t, 0, len(replaced.Requests()))
		equal(t, "other-model-2", other.Requests()[0].ModelID)
		equal(t, 0, other.Closes())
	})
}
func TestQueueCanBeReorderedAndEmptied(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		first, last := make(chan struct{}), make(chan struct{})
		f := newFixture(t, held(first, providertest.Text("one"), providertest.Response(1, 1)), said("three"), held(last, providertest.Text("two"), providertest.Response(1, 1)))
		c := f.opened()
		c.Send(t.Context(), send("m1", "one"))
		synctest.Wait()
		for _, id := range []string{"m2", "m3", "m4", "m5"} {
			c.Send(t.Context(), send(id, id))
		}
		c.Send(t.Context(), &framewire.SendQueuedMessageFrame{MessageID: turnID("m3")})
		c.Send(t.Context(), &framewire.DequeueMessageFrame{MessageID: turnID("m4")})
		queue := f.server.Tree(rootID()).Root().Session().QueuedMessages()
		equal(t, 3, len(queue))
		equal(t, turnID("m3"), queue[0].ID)
		close(first)
		synctest.Wait()
		equal(t, 3, len(f.script.Requests()))
		c.Send(t.Context(), &framewire.ClearMessageQueueFrame{})
		close(last)
		synctest.Wait()
		users := []core.TurnID{}
		for _, b := range f.server.Tree(rootID()).Root().Session().Transcript().Blocks {
			if u, ok := b.(*core.UserBlock); ok {
				users = append(users, u.TurnID)
			}
		}
		equal(t, []core.TurnID{turnID("m1"), turnID("m3"), turnID("m2")}, users)
		equal(t, 0, len(f.server.Tree(rootID()).Root().Session().QueuedMessages()))
	})
}
