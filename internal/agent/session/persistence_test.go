package session_test

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/session/sessiontest"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestToolsRunAfterDurableCallAndReuseID(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var f *scenario
		calls := 0
		stored := [][]string{}
		r := toolRuntime("look", func(_ context.Context, call session.ToolInvocation) (session.ToolOutcome, error) {
			cp := f.checkpoint()
			stored = append(stored, kinds(cp.Transcript))
			b := cp.Transcript[len(cp.Transcript)-2].(*core.ToolCallBlock)
			equal(t, b.Status, core.ToolCallStatus("executing"))
			equal(t, b.ToolUseID, call.ToolUseID)
			calls++
			return textOutcome(string(call.Input)), nil
		})
		f = start(t, r, session.DefaultConfig(), providertest.Events(providertest.ToolCall("call", "look", []byte(`{"value":"hello"}`)), providertest.Response(10, 5)), providertest.Events(providertest.ToolCall("call", "look", []byte(`{"value":"again"}`)), providertest.Response(15, 5)), answer("done"))
		f.done(f.send("look twice", "t1"))
		equal(t, calls, 2)
		f.history("user", "tool_call:completed", "response", "tool_call:completed", "response", "text", "response")
		equal(t, stored, [][]string{{"user", "tool_call:executing", "response"}, {"user", "tool_call:completed", "response", "tool_call:executing", "response"}})
		req := f.p.Requests()
		equal(t, itemKinds(req[2].Items), []string{"user_message", "tool_use", "tool_result", "tool_use", "tool_result"})
		for i, value := range []string{`{"value":"hello"}`, `{"value":"again"}`} {
			equal(t, req[2].Items[2+i*2], provider.InferenceItem(&provider.ToolResult{ToolUseID: "call", Output: []provider.ResultPart{&provider.TextPart{Text: value}}}))
		}
		equal(t, req[0].TurnID, "t1")
		for i := 1; i < len(req); i++ {
			equal(t, req[i].TurnID, req[0].TurnID)
			if req[i].RequestID == req[i-1].RequestID {
				t.Fatal("request ID reused")
			}
		}
	})
}
func TestDispatchAndFinalSaveTouchOnlyChangedRows(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		cfg := session.DefaultConfig()
		cfg.PersistInterval = time.Minute
		r := toolRuntime("look", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			return textOutcome("seen"), nil
		})
		f := start(t, r, cfg, tool("look"), tool("look"), answer("done"))
		f.done(f.send("go", "t1"))
		saves := f.tree.Saves()
		equal(t, len(saves), 4)
		want := [][]int{{0, 1, 2}, {1, 3, 4}, {3, 5, 6}}
		wantKinds := [][]string{{"user", "tool_call:executing", "response"}, {"tool_call:completed", "tool_call:executing", "response"}, {"tool_call:completed", "text", "response"}}
		for i, s := range saves[1:] {
			blocks := []core.Block{}
			indices := []int{}
			for _, b := range s.Update.ChangedBlocks {
				indices = append(indices, b.Index)
				blocks = append(blocks, b.Block)
			}
			equal(t, indices, want[i])
			equal(t, kinds(blocks), wantKinds[i])
		}
		equal(t, f.checkpoint().State.Phase, core.SessionPhase("idle"))
		equal(t, len(f.checkpoint().Transcript), 7)
		equal(t, saves[len(saves)-1].Update.BlockCount, 7)
		equal(t, saves[len(saves)-1].Update.State.Phase, core.SessionPhaseIdle)
	})
}
func TestFailedScheduledSaveRetainsDirtyRows(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		release := make(chan struct{})
		turn := func(ctx context.Context, r provider.InferenceRequest) provider.Run {
			return func(yield func(provider.Event) bool) {
				if !yield(providertest.Text("hello ")) {
					return
				}
				select {
				case <-release:
				case <-ctx.Done():
					return
				}
				for e := range providertest.Events(providertest.Text("world"), providertest.Response(1, 1))(ctx, r) {
					if !yield(e) {
						return
					}
				}
			}
		}
		f := start(t, &sessiontest.Runtime{}, session.DefaultConfig(), turn)
		f.tree.FailSaves(1)
		failed := make(chan struct{}, 1)
		errors := 0
		sub := f.s.Subscribe(func(e session.Event) {
			if _, ok := e.(*session.ErrorEvent); ok {
				errors++
				failed <- struct{}{}
			}
		})
		defer sub.Release()
		a := f.send("go", "t1")
		<-failed
		close(release)
		f.done(a)
		equal(t, kinds(f.checkpoint().Transcript), []string{"user", "text", "response"})
		equal(t, f.checkpoint().Transcript[1].(*core.TextBlock).Text, "hello world")
		equal(t, errors, 1)
		equal(t, f.checkpoint().Transcript, f.s.Transcript().Blocks)
	})
}
func TestFailingAndUnknownToolsAreResults(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r := toolRuntime("broken", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			return session.ToolOutcome{}, &session.ToolFailure{Message: "it broke"}
		})
		f := start(t, r, session.DefaultConfig(), providertest.Events(providertest.ToolCall("one", "broken", []byte(`{}`)), providertest.ToolCall("two", "missing", []byte(`{}`))), answer("done"))
		f.done(f.send("go", "t1"))
		f.history("user", "tool_call:error", "tool_call:error", "text", "response")
		for i, want := range []string{"Tool failed: it broke", "Tool not found: missing"} {
			b := f.s.Transcript().Blocks[i+1].(*core.ToolCallBlock)
			equal(t, b.Output, []core.ToolResultContentBlock{&core.ToolText{Text: want}})
		}
	})
}
func TestStopDuringToolAcknowledgesRecordedStop(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		entered := make(chan struct{})
		r := toolRuntime("slow", func(ctx context.Context, _ session.ToolInvocation) (session.ToolOutcome, error) {
			close(entered)
			<-ctx.Done()
			return session.ToolOutcome{}, ctx.Err()
		})
		f := start(t, r, session.DefaultConfig(), tool("slow"), answer("next"))
		a := f.send("go", "t1")
		<-entered
		result, err := f.s.Abort(t.Context())
		must(t, err)
		equal(t, *result.Target, framewire.AbortTargetActiveTool)
		equal(t, result.CanAbortAgain, false)
		equal(t, f.s.Transcript().Blocks[1].(*core.ToolCallBlock).Output, []core.ToolResultContentBlock{&core.ToolText{Text: "Tool call aborted: slow"}})
		f.history("user", "tool_call:error", "response", "abort")
		end, err := a.Wait(t.Context())
		must(t, err)
		equal(t, end, session.Aborted)
		equal(t, f.s.Phase(), core.SessionPhaseIdle)
		f.done(f.send("next", "t2"))
		equal(t, itemKinds(f.p.Requests()[1].Items), []string{"user_message", "tool_use", "tool_result", "user_message"})
		equal(t, f.p.Requests()[1].Items[2], provider.InferenceItem(&provider.ToolResult{ToolUseID: "call", IsError: true, Output: []provider.ResultPart{&provider.TextPart{Text: "Tool call aborted: slow"}}}))
	})
}
func TestStopDuringHangingHook(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		entered := make(chan struct{})
		r := &sessiontest.Runtime{Before: func(ctx context.Context) (*string, error) {
			close(entered)
			<-ctx.Done()
			return nil, ctx.Err()
		}}
		f := start(t, r, session.DefaultConfig())
		a := f.send("go", "t1")
		<-entered
		result, err := f.s.Abort(t.Context())
		must(t, err)
		equal(t, *result.Target, framewire.AbortTargetActiveTurn)
		end, err := a.Wait(t.Context())
		must(t, err)
		equal(t, end, session.Aborted)
		f.history("abort")
		equal(t, len(f.p.Requests()), 0)
	})
}
func TestDisposeDuringToolKeepsQueueAndInterruption(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		entered := make(chan struct{})
		closes := 0
		r := toolRuntime("slow", func(ctx context.Context, _ session.ToolInvocation) (session.ToolOutcome, error) {
			close(entered)
			<-ctx.Done()
			return session.ToolOutcome{}, ctx.Err()
		})
		r.Close = func(context.Context) error {
			closes++
			return nil
		}
		f := start(t, r, session.DefaultConfig(), tool("slow"))
		a := f.send("go", "t1")
		<-entered
		q := f.send("later", "t2")
		must(t, f.s.Dispose(t.Context()))
		end, err := a.Wait(t.Context())
		must(t, err)
		equal(t, end, session.Aborted)
		end, err = q.Wait(t.Context())
		must(t, err)
		equal(t, end, session.Detached)
		cp := f.checkpoint()
		equal(t, cp.State.Phase, core.SessionPhase("running"))
		equal(t, len(cp.State.Queue), 1)
		equal(t, cp.State.Queue[0].ID, core.TurnID("t2"))
		equal(t, cp.Transcript[1].(*core.ToolCallBlock).Output, []core.ToolResultContentBlock{&core.ToolText{Text: "Tool call aborted: slow"}})
		record := cp.Transcript[3].(*core.ErrorBlock)
		equal(t, record.Code, new("interrupted"))
		equal(t, record.Message, "The agent session was shut down while this turn was running.")
		equal(t, kinds(cp.Transcript), []string{"user", "tool_call:error", "response", "error"})
		equal(t, closes, 1)
		equal(t, f.p.Closes(), 1)
		_, err = f.s.Send(storetest.Text("no"), "t3")
		equal(t, err, error(session.AdmissionClosed))
	})
}
func TestRestoreExecutingToolAsInterrupted(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		entered := make(chan struct{})
		r := toolRuntime("slow", func(ctx context.Context, _ session.ToolInvocation) (session.ToolOutcome, error) {
			close(entered)
			<-ctx.Done()
			return session.ToolOutcome{}, ctx.Err()
		})
		f := start(t, r, session.DefaultConfig(), tool("slow"))
		f.send("go", "t1")
		<-entered
		cp := f.checkpoint()
		must(t, f.s.Dispose(t.Context()))
		calls := 0
		restoredRuntime := toolRuntime("slow", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			calls++
			return textOutcome("rerun"), nil
		})
		s, c, p := f.restoreWith(cp, restoredRuntime, core.SystemClock{}, answer("next"))
		equal(t, c.Interrupted, true)
		equal(t, s.Phase(), core.SessionPhase("idle"))
		equal(t, kinds(s.Transcript().Blocks), []string{"user", "tool_call:error", "response"})
		equal(t, cp.State.Phase, core.SessionPhaseRunning)
		equal(t, s.Transcript().Blocks[1].(*core.ToolCallBlock).Output, []core.ToolResultContentBlock{&core.ToolText{Text: "Tool call interrupted: slow (the process died before a result was recorded)"}})
		a, err := s.Send(storetest.Text("next"), "t2")
		must(t, err)
		_, err = a.Wait(t.Context())
		must(t, err)
		equal(t, len(p.Requests()), 1)
		equal(t, p.Requests()[0].Items[2].(*provider.ToolResult).IsError, true)
		equal(t, itemKinds(p.Requests()[0].Items), []string{"user_message", "tool_use", "tool_result", "user_message"})
		equal(t, calls, 0)
	})
}
func TestScheduledSaveAndFlushAreSerialized(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		turn, entered, release := gated(providertest.Text("done"), providertest.Response(1, 1))
		f := start(t, &sessiontest.Runtime{}, session.DefaultConfig(), turn)
		gate := f.tree.HoldSaves()
		defer gate.Release()
		a := f.send("go", "t1")
		<-entered
		time.Sleep(time.Second)
		must(t, gate.Wait(t.Context(), 1))
		close(release)
		synctest.Wait()
		equal(t, gate.Waiting(), 1)
		gate.Release()
		f.done(a)
		equal(t, len(f.tree.Saves()), 3)
		last := f.tree.Saves()[2].Update
		equal(t, last.BlockCount, len(f.s.Transcript().Blocks))
		equal(t, last.State.Phase, core.SessionPhaseIdle)
		equal(t, f.checkpoint().State.Phase, core.SessionPhase("idle"))
	})
}
func TestQueuedMessageSavedWithoutTranscriptChange(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		entered, release := make(chan struct{}), make(chan struct{})
		r := toolRuntime("hold", func(ctx context.Context, _ session.ToolInvocation) (session.ToolOutcome, error) {
			close(entered)
			select {
			case <-release:
				return textOutcome("ok"), nil
			case <-ctx.Done():
				return session.ToolOutcome{}, ctx.Err()
			}
		})
		f := start(t, r, session.DefaultConfig(), tool("hold"), answer("first"), answer("queued"))
		a := f.send("go", "t1")
		<-entered
		before := len(f.tree.Saves())
		q := f.send("later", "t2")
		time.Sleep(2 * time.Second)
		synctest.Wait()
		saves := f.tree.Saves()
		last := saves[len(saves)-1].Update
		equal(t, len(last.ChangedBlocks), 0)
		equal(t, len(last.State.Queue), 1)
		equal(t, last.State.Queue[0].ID, core.TurnID("t2"))
		equal(t, len(saves)-before, 1)
		close(release)
		f.done(a)
		f.done(q)
	})
}
func TestProviderSwitchContinuesRunningTurn(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		entered, release := make(chan struct{}), make(chan struct{})
		r := toolRuntime("hold", func(ctx context.Context, _ session.ToolInvocation) (session.ToolOutcome, error) {
			close(entered)
			select {
			case <-release:
				return textOutcome("ok"), nil
			case <-ctx.Done():
				return session.ToolOutcome{}, ctx.Err()
			}
		})
		f := start(t, r, session.DefaultConfig(), tool("hold"))
		a := f.send("go", "t1")
		<-entered
		next := providertest.NewScriptedRuntime(t, answer("new provider"))
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: storetest.ModelOf("other", "model-b"), Runtime: next}))
		close(release)
		f.done(a)
		equal(t, f.p.Closes(), 1)
		equal(t, next.Requests()[0].ModelID, "model-b")
		equal(t, next.Requests()[0].TurnID, "t1")
		equal(t, len(next.Requests()[0].Items), 3)
		equal(t, len(f.p.Requests()), 1)
		equal(t, itemKinds(next.Requests()[0].Items), []string{"user_message", "tool_use", "tool_result"})
	})
}
func TestStreamBlocksAndDeltaPatches(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, providertest.Events(&provider.ThinkingStart{}, providertest.Thinking("private "), providertest.Thinking("notes"), &provider.ThinkingSignature{Signature: "anthropic:sig"}, &provider.RedactedThinking{Data: "opaque"}, providertest.Text("Hello "), providertest.Text("world"), providertest.ToolCall("call", "echo", []byte(`"{\"broken\":"`)), providertest.Response(1, 1)), answer("done"))
		deltas := []string{}
		sub := f.s.Subscribe(func(e session.Event) {
			if e, ok := e.(*session.TranscriptChanged); ok {
				for _, p := range e.Patches {
					if p, ok := p.(*framewire.AppendTextPatch); ok {
						deltas = append(deltas, p.Delta)
					}
				}
			}
		})
		defer sub.Release()
		f.done(f.send("hello", "t1"))
		f.history("user", "thinking", "redacted_thinking", "text", "tool_call:error", "response", "text", "response")
		equal(t, deltas, []string{"notes", "world"})
		blocks := f.s.Transcript().Blocks
		equal(t, blocks[1].(*core.ThinkingBlock).Text, "private notes")
		equal(t, *blocks[1].(*core.ThinkingBlock).Signature, "anthropic:sig")
		equal(t, blocks[3].(*core.TextBlock).Forkable, true)
		equal(t, blocks[3].(*core.TextBlock).Text, "Hello world")
		equal(t, blocks[4].(*core.ToolCallBlock).Input, `{"broken":`)
		equal(t, blocks[4].(*core.ToolCallBlock).Output, []core.ToolResultContentBlock{&core.ToolText{Text: "Tool not found: echo"}})
		replayed := f.p.Requests()[1].Items
		equal(t, replayed[1:3], []provider.InferenceItem{&provider.AssistantThinking{ModelID: "test-model", Text: "private notes", Signature: new("anthropic:sig")}, &provider.AssistantRedactedThinking{ModelID: "test-model", Data: "opaque"}})
		equal(t, replayed[4].(*provider.ToolUse).Input, json.RawMessage(`"{\"broken\":"`))
	})
}
func TestLongHistoryDeltaAndSaveScope(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, answer("short"))
		f.done(f.send("begin", "t0"))
		cp := f.checkpoint()
		original := cp.Transcript
		for i := range 1000 {
			for j, b := range original {
				id := core.BlockID(fmt.Sprintf("history-%d-%d", i, j))
				switch b := b.(type) {
				case *core.UserBlock:
					c := *b
					c.BlockID = id
					cp.Transcript = append(cp.Transcript, &c)
				case *core.TextBlock:
					c := *b
					c.BlockID = id
					cp.Transcript = append(cp.Transcript, &c)
				case *core.ResponseBlock:
					c := *b
					c.BlockID = id
					cp.Transcript = append(cp.Transcript, &c)
				case *core.AbortBlock, *core.AgentMessageBlock, *core.CompactionBoundaryBlock, *core.CompactionMarkerBlock, *core.ContextBlock, *core.ErrorBlock, *core.RedactedThinkingBlock, *core.ResumeBlock, *core.SteerBlock, *core.ThinkingBlock, *core.ToolCallBlock, *core.WakeupBlock:
					t.Fatalf("unexpected seed block %T", b)
				}
			}
		}
		events := []provider.Event{}
		for i := range 200 {
			events = append(events, providertest.Text(fmt.Sprintf("d%d ", i)))
		}
		events = append(events, providertest.Response(1, 1))
		s, _, _ := f.restore(cp, providertest.Events(events...))
		patches := []framewire.TranscriptPatch{}
		sub := s.Subscribe(func(e session.Event) {
			if e, ok := e.(*session.TranscriptChanged); ok {
				patches = append(patches, e.Patches...)
			}
		})
		defer sub.Release()
		before := len(f.tree.Saves())
		a, err := s.Send(storetest.Text("stream"), "t1")
		must(t, err)
		_, err = a.Wait(t.Context())
		must(t, err)
		var appended strings.Builder
		for _, p := range patches {
			var index uint32
			switch p := p.(type) {
			case *framewire.AddPatch:
				index = p.Index
			case *framewire.AppendTextPatch:
				index = p.Index
				appended.WriteString(p.Delta)
			case *framewire.ReplaceBlockPatch:
				index = p.Index
			case *framewire.ReplacePatch:
				t.Fatal("history replaced")
			}
			if int(index) < len(cp.Transcript) {
				t.Fatal("history touched")
			}
		}
		var expected strings.Builder
		for i := 1; i < 200; i++ {
			fmt.Fprintf(&expected, "d%d ", i)
		}
		equal(t, appended.String(), expected.String())
		for _, save := range f.tree.Saves()[before:] {
			for _, row := range save.Update.ChangedBlocks {
				if row.Index < len(cp.Transcript) {
					t.Fatal("history saved")
				}
			}
		}
	})
}
func TestPromptCacheAnsweredItems(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r := toolRuntime("look", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			return textOutcome("seen"), nil
		})
		f := start(t, r, session.DefaultConfig(), providertest.Events(&provider.ThinkingStart{}, providertest.Thinking("plan"), providertest.Text("looking twice"), providertest.ToolCall("one", "look", []byte(`{}`)), providertest.ToolCall("two", "look", []byte(`{}`)), providertest.Response(1, 1)), answer("done"), answer("again"))
		f.done(f.send("look twice", "t1"))
		f.done(f.send("again", "t2"))
		req := f.p.Requests()
		equal(t, len(req), 3)
		equal(t, itemKinds(req[1].Items), []string{"user_message", "assistant_thinking", "assistant_text", "tool_use", "tool_result", "tool_use", "tool_result"})
		for i, want := range []int{0, 1, 7} {
			equal(t, *req[i].PromptCache.AnsweredItems, want)
		}
		equal(t, req[1].Items[:1], req[0].Items)
		equal(t, req[2].Items[:7], req[1].Items)
	})
}
func TestStopRunningThenQueuedThenNothing(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, providertest.Pending(), answer("third"))
		a := f.send("hang", "t1")
		b := f.send("queued", "t2")
		c := f.send("third", "t3")
		synctest.Wait()
		gate := f.tree.HoldSaves()
		defer gate.Release()
		first, err := f.s.Abort(t.Context())
		must(t, err)
		equal(t, *first.Target, framewire.AbortTargetActiveProviderStream)
		equal(t, first.CanAbortAgain, true)
		second, err := f.s.Abort(t.Context())
		must(t, err)
		equal(t, *second.Target, framewire.AbortTargetQueuedMessage)
		equal(t, second.CanAbortAgain, true)
		gate.Release()
		end, err := a.Wait(t.Context())
		must(t, err)
		equal(t, end, session.Aborted)
		end, err = b.Wait(t.Context())
		must(t, err)
		equal(t, end, session.Dropped)
		f.done(c)
		f.history("user", "abort", "user", "text", "response")
		last, err := f.s.Abort(t.Context())
		must(t, err)
		equal(t, last, framewire.AbortResult{})
	})
}
