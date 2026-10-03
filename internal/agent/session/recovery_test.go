package session_test

import (
	"context"
	"errors"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript/transcripttest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestTransientFailuresBeforeOutput(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, providertest.Events(providertest.Error("overloaded", new(provider.Overloaded))), providertest.Events(&provider.Error{Failure: provider.Failure{Message: "limited", Code: new(provider.RateLimit), RetryAfter: new(1500 * time.Millisecond)}}), answer("done"))
		reports := []session.RetryScheduled{}
		sub := f.s.Subscribe(func(e session.Event) {
			if e, ok := e.(*session.RetryScheduled); ok {
				reports = append(reports, *e)
			}
		})
		defer sub.Release()
		f.done(f.send("go", "t1"))
		f.history("user", "text", "response")
		equal(t, len(f.p.Requests()), 3)
		equal(t, len(reports), 2)
		equal(t, reports[0].Attempt, uint32(1))
		equal(t, reports[1].Attempt, uint32(2))
		equal(t, reports[1].DelayMS, uint64(1500))
		equal(t, reports[0].Code, new("overloaded"))
		equal(t, reports[1].Code, new("rate_limit"))
		if reports[0].DelayMS >= 1000 {
			t.Fatal(reports[0])
		}
		for i, r := range reports {
			equal(t, *r.Diagnostics.ClientRequestID, f.p.Requests()[i].RequestID)
			equal(t, r.Diagnostics.Source, "unknown")
		}
	})
}
func TestThinkingUnwindsButTextMakesFailureTerminal(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, providertest.Events(&provider.ThinkingStart{}, providertest.Thinking("plan"), providertest.Error("busy", new(provider.Overloaded))), answer("done"), providertest.Events(providertest.Text("partial"), providertest.Error("busy", new(provider.Overloaded))))
		f.done(f.send("one", "t1"))
		assertActionCode(t, f.send("two", "t2"), "overloaded")
		f.history("user", "text", "response", "user", "text", "error")
		equal(t, len(f.p.Requests()), 3)
	})
}
func TestTransientAttemptAndVendorWaitLimits(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		turns := []providertest.Turn{}
		for range 4 {
			turns = append(turns, providertest.Events(providertest.Error("busy", new(provider.Overloaded))))
		}
		turns = append(turns, providertest.Events(&provider.Error{Failure: provider.Failure{Message: "wait", Code: new(provider.RateLimit), RetryAfter: new(31 * time.Second)}}))
		f := setup(t, turns...)
		reports := []session.RetryScheduled{}
		sub := f.s.Subscribe(func(e session.Event) {
			if e, ok := e.(*session.RetryScheduled); ok {
				reports = append(reports, *e)
			}
		})
		defer sub.Release()
		f.fail(f.send("first", "t1"))
		assertActionCode(t, f.send("second", "t2"), "rate_limit")
		equal(t, len(reports), 3)
		for i, r := range reports {
			equal(t, r.Attempt, uint32(i+1))
			if r.DelayMS >= uint64(1000<<i) {
				t.Fatal(r)
			}
		}
		f.history("user", "error", "user", "error")
		equal(t, len(f.p.Requests()), 5)
	})
}
func TestTransientFailureAfterToolDoesNotRepeatTool(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		calls := 0
		r := toolRuntime("look", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			calls++
			return textOutcome("seen"), nil
		})
		f := start(t, r, session.DefaultConfig(), tool("look"), providertest.Events(providertest.Error("limited", new(provider.RateLimit))), answer("done"))
		reports := 0
		sub := f.s.Subscribe(func(e session.Event) {
			if _, ok := e.(*session.RetryScheduled); ok {
				reports++
			}
		})
		defer sub.Release()
		f.done(f.send("look", "t1"))
		equal(t, calls, 1)
		requests := f.p.Requests()
		equal(t, len(requests), 3)
		equal(t, requests[1].Items, requests[2].Items)
		equal(t, reports, 1)
		f.history("user", "tool_call:completed", "response", "text", "response")
	})
}
func TestResumeAfterToolFailureKeepsResult(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		calls := 0
		r := toolRuntime("look", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			calls++
			return textOutcome("seen"), nil
		})
		f := start(t, r, session.DefaultConfig(), tool("look"), providertest.Events(&provider.ThinkingStart{}, providertest.Thinking("plan"), providertest.Error("failure", nil)), answer("done"))
		f.fail(f.send("look", "t1"))
		f.history("user", "tool_call:completed", "response", "thinking", "error")
		a, err := f.s.Resume()
		must(t, err)
		f.done(a)
		equal(t, calls, 1)
		f.history("user", "tool_call:completed", "response", "resume", "text", "response")
		last := f.p.Requests()[2].Items
		equal(t, len(last), 4)
		if _, ok := last[0].(*provider.UserMessage); !ok {
			t.Fatalf("first item: %T", last[0])
		}
		if _, ok := last[1].(*provider.ToolUse); !ok {
			t.Fatalf("second item: %T", last[1])
		}
		if _, ok := last[2].(*provider.ToolResult); !ok {
			t.Fatalf("third item: %T", last[2])
		}
		equal(t, last[len(last)-1], provider.InferenceItem(&provider.UserMessage{Content: storetest.SentText(transcripttest.ResumeText)}))
	})
}
func TestResumeEmptyTurnRerunsWithoutResumeBlock(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, providertest.Events(providertest.Error("failure", nil)), answer("done"))
		f.fail(f.send("look", "t1"))
		a, err := f.s.Resume()
		must(t, err)
		f.done(a)
		f.history("user", "text", "response")
		requests := f.p.Requests()
		equal(t, requests[0].Items, requests[1].Items)
		equal(t, requests[1].TurnID, "t1")
	})
}
func TestRetryKeepsTurnSteers(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		turn, entered, release := gated(providertest.Text("first"), providertest.Response(1, 1))
		f := setup(t, turn, answer("noted"), answer("replacement"))
		a := f.send("look", "t1")
		<-entered
		must(t, f.s.Steer(storetest.Text("be brief"), "s1"))
		close(release)
		f.done(a)
		patches := []framewire.TranscriptPatch{}
		sub := f.s.Subscribe(func(e session.Event) {
			if e, ok := e.(*session.TranscriptChanged); ok {
				patches = append(patches, e.Patches...)
			}
		})
		defer sub.Release()
		a, err := f.s.Retry()
		must(t, err)
		f.done(a)
		f.history("user", "steer", "text", "response")
		equal(t, f.p.Requests()[2].TurnID, "t1")
		equal(t, f.s.Transcript().Blocks[1].(*core.SteerBlock).TurnID, core.TurnID("t1"))
		items := f.p.Requests()[2].Items
		equal(t, len(items), 2)
		if _, ok := items[0].(*provider.UserMessage); !ok {
			t.Fatalf("first item: %T", items[0])
		}
		if _, ok := items[1].(*provider.UserSteer); !ok {
			t.Fatalf("second item: %T", items[1])
		}
		replace, ok := patches[0].(*framewire.ReplacePatch)
		if !ok {
			t.Fatalf("first patch %T", patches[0])
		}
		equal(t, kinds(replace.Value), []string{"user", "steer"})
		equal(t, kinds(f.checkpoint().Transcript), kinds(f.s.Transcript().Blocks))
	})
}
func TestResumeMarksStopAndContinues(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, hanging(providertest.Text("partial")), answer("done"))
		a := f.send("go", "t1")
		synctest.Wait()
		must(t, f.s.Steer(storetest.Text("brief"), "s1"))
		_, err := f.s.Abort(t.Context())
		must(t, err)
		end, err := a.Wait(t.Context())
		must(t, err)
		equal(t, end, session.Aborted)
		a, err = f.s.Resume()
		must(t, err)
		f.done(a)
		f.history("user", "text", "steer", "abort", "resume", "text", "response")
		blocks := f.s.Transcript().Blocks
		abort := blocks[3].(*core.AbortBlock)
		equal(t, abort.IsResumed, true)
		equal(t, f.p.Requests()[1].Items, []provider.InferenceItem{
			&provider.UserMessage{Content: storetest.SentText("go")},
			&provider.AssistantText{ModelID: "test-model", Text: "partial"},
			&provider.UserSteer{Content: storetest.SentText("brief")},
			&provider.UserMessage{Content: storetest.SentText(transcripttest.ResumeText)},
		})
	})
}
func TestStopDuringResumeSave(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, providertest.Events(providertest.Text("partial"), providertest.Error("failure", nil)))
		f.fail(f.send("go", "t1"))
		gate := f.tree.HoldSaves()
		defer gate.Release()
		a, err := f.s.Resume()
		must(t, err)
		must(t, gate.Wait(t.Context(), 1))
		result := make(chan framewire.AbortResult, 1)
		go func() {
			r, err := f.s.Abort(t.Context())
			if err != nil {
				t.Error(err)
			}
			result <- r
		}()
		synctest.Wait()
		select {
		case <-result:
			t.Fatal("stop answered before unwind save committed")
		default:
		}
		gate.Release()
		r := <-result
		equal(t, *r.Target, framewire.AbortTargetActiveTurn)
		end, err := a.Wait(t.Context())
		must(t, err)
		equal(t, end, session.Aborted)
		f.history("user", "text", "abort")
		equal(t, len(f.p.Requests()), 1)
	})
}

// assertActionCode checks the failure class returned by a session action.
func assertActionCode(t *testing.T, action *session.ActionHandle, code string) {
	t.Helper()
	_, err := action.Wait(t.Context())
	var report *session.ErrorReport
	if !errors.As(err, &report) {
		t.Fatalf("action failure: %v", err)
	}
	equal(t, report.Code, new(code))
}
