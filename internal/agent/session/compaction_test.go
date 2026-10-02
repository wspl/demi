package session_test

import (
	"context"
	"reflect"
	"slices"
	"strings"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/session/sessiontest"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/agent/transcript/transcripttest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func unmeasured(text string) providertest.Turn {
	return providertest.Events(providertest.Text(text), providertest.Response(0, 0))
}
func tooLong() providertest.Turn {
	return providertest.Events(providertest.Error("prompt is too long", new(provider.ContextLengthExceeded)))
}
func smallModel() core.ModelSelection {
	m := storetest.ModelOf("stub", "small-model")
	m.Model.ContextWindow = 1000
	return m
}
func small(t *testing.T, automatic bool, turns ...providertest.Turn) *scenario {
	cfg := session.DefaultConfig()
	if !automatic {
		cfg.Compaction.ThresholdPercent = nil
	}
	f := start(t, &sessiontest.Runtime{}, cfg, turns...)
	must(t, f.s.UpdateModel(session.ModelSwitch{Model: smallModel()}))
	return f
}
func userItem(text string) provider.InferenceItem {
	return &provider.UserMessage{Content: storetest.SentText(text)}
}
func summaryItem(text string) provider.InferenceItem {
	return userItem("Previous conversation summary:\n" + text)
}
func assistantItem(model, text string) provider.InferenceItem {
	return &provider.AssistantText{ModelID: model, Text: text}
}
func isCopy(r provider.InferenceRequest) bool {
	return slices.ContainsFunc(r.Items, func(i provider.InferenceItem) bool {
		return reflect.DeepEqual(i, userItem(sessiontest.CompactionSummaryInstruction))
	})
}
func summarySizes(p *providertest.ScriptedRuntime) []int {
	sizes := []int{}
	for _, r := range p.Requests() {
		if isCopy(r) {
			sizes = append(sizes, len(r.Items))
		}
	}
	return sizes
}
func compact(t *testing.T, s *session.Session) {
	t.Helper()
	a, err := s.Compact()
	must(t, err)
	_, err = a.Wait(t.Context())
	must(t, err)
}
func countKind(s *session.Session, kind string) int {
	n := 0
	for _, k := range kinds(s.Transcript().Blocks) {
		if k == kind {
			n++
		}
	}
	return n
}
func TestThresholdCompactionCopyRepeatsPrefix(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := small(t, true, unmeasured("first answer"), unmeasured("  summary of first\n"), unmeasured("second"))
		f.done(f.send(strings.Repeat("x", 3000), "t1"))
		f.done(f.send(strings.Repeat("y", 400), "t2"))
		r := f.p.Requests()
		equal(t, len(r), 3)
		equal(t, r[1].Items, append(slices.Clone(r[0].Items), userItem(sessiontest.CompactionSummaryInstruction)))
		equal(t, r[1].SessionID, r[0].SessionID)
		equal(t, r[1].ModelID, "small-model")
		equal(t, r[1].SystemPrompt, r[2].SystemPrompt)
		equal(t, r[2].Items, []provider.InferenceItem{summaryItem("summary of first"), assistantItem("small-model", "first answer"), userItem(strings.Repeat("y", 400))})
		equal(t, f.p.Closes(), 1)
		f.history("user", "compaction_boundary", "text", "response", "user", "compaction_marker", "text", "response")
	})
}
func TestRequestPrefixesRestartAtSummaryAndSurviveRestore(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := small(t, false, unmeasured("one"), unmeasured("two"), unmeasured("summary"), unmeasured("three"), unmeasured("four"))
		f.done(f.send("one", "t1"))
		f.done(f.send("two", "t2"))
		compact(t, f.s)
		cp := f.checkpoint()
		f.done(f.send("three", "t3"))
		f.done(f.send("four", "t4"))
		r := f.p.Requests()
		equal(t, r[1].Items[:len(r[0].Items)], r[0].Items)
		equal(t, r[2].Items, append(slices.Clone(r[1].Items), userItem(sessiontest.CompactionSummaryInstruction)))
		equal(t, r[3].Items[0], summaryItem("summary"))
		equal(t, r[4].Items[:len(r[3].Items)], r[3].Items)
		s, _, p := f.restore(cp, unmeasured("three"))
		a, err := s.Send(storetest.Text("three"), "t3")
		must(t, err)
		_, err = a.Wait(t.Context())
		must(t, err)
		equal(t, p.Requests()[0].Items, r[3].Items)
		for i, request := range []provider.InferenceRequest{r[0], r[1], r[3], r[4]} {
			expected := []int{0, len(r[0].Items), 0, len(r[3].Items)}
			equal(t, *request.PromptCache.AnsweredItems, expected[i])
		}
	})
}
func TestSummaryOverflowHalvesWindow(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := small(t, true, unmeasured("one"), unmeasured("two"), tooLong(), unmeasured("half"), unmeasured("done"))
		f.done(f.send(strings.Repeat("a", 1200), "t1"))
		f.done(f.send(strings.Repeat("b", 1200), "t2"))
		f.done(f.send(strings.Repeat("c", 800), "t3"))
		equal(t, summarySizes(f.p), []int{4, 3})
		equal(t, countKind(f.s, "compaction_boundary"), 1)
		if _, ok := f.s.Transcript().Blocks[2].(*core.CompactionBoundaryBlock); !ok {
			t.Fatal("wrong cut")
		}
	})
}
func TestBlankAndFailedSummaryLeaveNoBoundary(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := small(t, true, unmeasured("first"), unmeasured("  "), unmeasured("second"), providertest.Events(providertest.Error("expired", new(provider.AuthExpired))))
		f.done(f.send(strings.Repeat("x", 3000), "t1"))
		f.done(f.send(strings.Repeat("y", 400), "t2"))
		f.fail(f.send(strings.Repeat("z", 400), "t3"))
		f.history("user", "text", "response", "user", "text", "response", "user")
	})
}
func TestStopSummaryClosesCopy(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := small(t, true, unmeasured("first"), providertest.Pending())
		f.done(f.send(strings.Repeat("x", 3000), "t1"))
		a := f.send(strings.Repeat("y", 400), "t2")
		synctest.Wait()
		equal(t, f.s.Phase(), core.SessionPhase("compacting"))
		r, err := f.s.Abort(t.Context())
		must(t, err)
		equal(t, *r.Target, framewire.AbortTargetActiveCompaction)
		end, err := a.Wait(t.Context())
		must(t, err)
		equal(t, end, session.Aborted)
		equal(t, f.p.Closes(), 1)
		equal(t, len(f.p.Requests()), 2)
		equal(t, countKind(f.s, "compaction_boundary"), 0)
	})
}
func TestCompactionToolsDoNotChangeParent(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		calls := 0
		r := toolRuntime("look", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			calls++
			return textOutcome("seen"), nil
		})
		f := start(t, r, session.DefaultConfig(), unmeasured("first"), tool("look"), unmeasured("summary after tool"), unmeasured("second"))
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: smallModel()}))
		f.done(f.send(strings.Repeat("x", 3000), "t1"))
		f.done(f.send(strings.Repeat("y", 400), "t2"))
		equal(t, calls, 1)
		equal(t, countKind(f.s, "tool_call:completed"), 0)
		equal(t, f.s.Transcript().Blocks[1].(*core.CompactionBoundaryBlock).Summary, "summary after tool")
	})
}
func TestUsageCompactsAtMostThreeTimes(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		round := providertest.Events(providertest.ToolCall("call", "work", []byte(`{}`)), providertest.Response(900, 0))
		r := toolRuntime("work", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			return textOutcome(strings.Repeat("w", 800)), nil
		})
		f := start(t, r, session.DefaultConfig(), unmeasured("first"), round, unmeasured("one"), round, unmeasured("two"), round, unmeasured("three"), round, providertest.Events(providertest.Text("done"), providertest.Response(900, 0)))
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: smallModel()}))
		f.done(f.send(strings.Repeat("x", 3000), "t1"))
		f.done(f.send("go on", "t2"))
		equal(t, countKind(f.s, "compaction_boundary"), 3)
		equal(t, countKind(f.s, "resume"), 3)
		equal(t, f.p.Remaining(), 0)
	})
}
func TestSmallerModelSwitchCompactsWithOldModel(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, unmeasured("first"), unmeasured("second"), unmeasured("summary"), unmeasured("third"), unmeasured("fourth"))
		f.done(f.send(strings.Repeat("a", 1600), "t1"))
		f.done(f.send(strings.Repeat("b", 1600), "t2"))
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: smallModel()}))
		f.done(f.send("short", "t3"))
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: storetest.TestModel()}))
		f.done(f.send("more", "t4"))
		r := f.p.Requests()
		equal(t, len(r), 5)
		equal(t, r[2].ModelID, "test-model")
		equal(t, isCopy(r[2]), true)
		equal(t, r[3].ModelID, "small-model")
		equal(t, r[4].ModelID, "test-model")
	})
}
func TestSmallerModelSwitchDuringTool(t *testing.T) {
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
		f := start(t, r, session.DefaultConfig(), unmeasured("first"), providertest.Events(providertest.ToolCall("call", "hold", []byte(`{}`)), providertest.Response(0, 0)), unmeasured("summary"), unmeasured("continued"))
		f.done(f.send(strings.Repeat("a", 3600), "t1"))
		a := f.send(strings.Repeat("b", 480), "t2")
		<-entered
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: smallModel()}))
		close(release)
		f.done(a)
		req := f.p.Requests()
		equal(t, len(req), 4)
		equal(t, req[2].ModelID, "test-model")
		equal(t, req[3].ModelID, "small-model")
		equal(t, req[3].Items[0], summaryItem("summary"))
		equal(t, req[3].Items[len(req[3].Items)-1], userItem(transcripttest.ResumeText))
		equal(t, req[3].TurnID, "t2")
	})
}
func TestCompactWritesSteerButContinuesOnlyForAgents(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		first, entered, release := gated(providertest.Text("summary one"), providertest.Response(0, 0))
		second, entered2, release2 := gated(providertest.Text("summary two"), providertest.Response(0, 0))
		f := small(t, false, unmeasured("one"), unmeasured("two"), first, unmeasured("three"), second, unmeasured("read"))
		f.done(f.send("one", "t1"))
		f.done(f.send("two", "t2"))
		a, err := f.s.Compact()
		must(t, err)
		<-entered
		must(t, f.s.Steer(storetest.Text("mind"), "s1"))
		close(release)
		f.done(a)
		equal(t, len(f.p.Requests()), 3)
		equal(t, kinds(f.s.Transcript().Blocks)[len(f.s.Transcript().Blocks)-1], "steer")
		f.done(f.send("three", "t3"))
		a, err = f.s.Compact()
		must(t, err)
		<-entered2
		must(t, f.s.AcceptAgentMessage(t.Context(), message("news")))
		close(release2)
		f.done(a)
		equal(t, len(f.p.Requests()), 6)
		equal(t, steers(f.p.Requests()[5]), []string{transcripttest.AgentMessageEnvelope(message("news"))})
	})
}
func TestSummaryOverflowToOneBlockLeavesHistory(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := small(t, false, unmeasured("old"), unmeasured("recent"), tooLong(), tooLong(), tooLong(), unmeasured("recovered"), unmeasured("summary"), unmeasured("fourth"), tooLong(), tooLong())
		f.done(f.send(strings.Repeat("x", 3000), "t1"))
		f.done(f.send(strings.Repeat("y", 400), "t2"))
		before := f.s.Transcript()
		a, err := f.s.Compact()
		must(t, err)
		f.fail(a)
		equal(t, f.s.Transcript(), before)
		equal(t, summarySizes(f.p), []int{4, 3, 2})
		f.done(f.send("recover", "t3"))
		compact(t, f.s)
		f.done(f.send(strings.Repeat("z", 400), "t4"))
		before = f.s.Transcript()
		a, err = f.s.Compact()
		must(t, err)
		f.fail(a)
		equal(t, f.s.Transcript(), before)
		equal(t, summarySizes(f.p), []int{4, 3, 2, 6, 4, 3})
	})
}
func TestRetryCompactionAndStop(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		question := "old question " + strings.Repeat("x", 3200)
		f := small(t, false, unmeasured("old answer"), unmeasured("bad answer"))
		f.done(f.send(question, "t1"))
		f.done(f.send("retry this", "t2"))
		cp := f.checkpoint()
		f.config.Compaction = session.DefaultCompactionConfig()
		s, _, p := f.restore(cp, unmeasured("retry summary"), unmeasured("retried"))
		a, err := s.Retry()
		must(t, err)
		_, err = a.Wait(t.Context())
		must(t, err)
		equal(t, p.Requests()[0].Items, []provider.InferenceItem{userItem(question), userItem(sessiontest.CompactionSummaryInstruction)})
		equal(t, p.Requests()[1].Items, []provider.InferenceItem{summaryItem("retry summary"), assistantItem("small-model", "old answer"), userItem("retry this")})
		equal(t, p.Requests()[1].TurnID, "t2")
		stopped, _, pp := f.restore(cp, providertest.Pending())
		a, err = stopped.Retry()
		must(t, err)
		synctest.Wait()
		r, err := stopped.Abort(t.Context())
		must(t, err)
		equal(t, *r.Target, framewire.AbortTargetActiveCompaction)
		end, err := a.Wait(t.Context())
		must(t, err)
		equal(t, end, session.Aborted)
		equal(t, kinds(stopped.Transcript().Blocks), []string{"user", "text", "response", "user", "abort"})
		equal(t, len(pp.Requests()), 1)
	})
}
func TestResumeCompactionKeepsStoppedInputAndStop(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		old := "old answer " + strings.Repeat("a", 1200)
		question := "question " + strings.Repeat("q", 2400)
		partial := "partial " + strings.Repeat("y", 300)
		f := small(t, false, unmeasured(old), unmeasured("summary one"), hanging(providertest.Text(partial)))
		f.done(f.send("old question", "t1"))
		compact(t, f.s)
		a := f.send(question, "t2")
		synctest.Wait()
		_, err := f.s.Abort(t.Context())
		must(t, err)
		_, err = a.Wait(t.Context())
		must(t, err)
		cp := f.checkpoint()
		f.config.Compaction = session.DefaultCompactionConfig()
		s, _, p := f.restore(cp, unmeasured("resume summary"), unmeasured("continued"))
		a, err = s.Resume()
		must(t, err)
		_, err = a.Wait(t.Context())
		must(t, err)
		equal(t, p.Requests()[0].Items, []provider.InferenceItem{summaryItem("summary one"), assistantItem("small-model", old), userItem(sessiontest.CompactionSummaryInstruction)})
		equal(t, p.Requests()[1].Items, []provider.InferenceItem{summaryItem("resume summary"), userItem(question), assistantItem("small-model", partial), userItem(transcripttest.ResumeText)})
		stopped, _, pp := f.restore(cp, providertest.Pending())
		a, err = stopped.Resume()
		must(t, err)
		synctest.Wait()
		r, err := stopped.Abort(t.Context())
		must(t, err)
		equal(t, *r.Target, framewire.AbortTargetActiveCompaction)
		_, err = a.Wait(t.Context())
		must(t, err)
		equal(t, countKind(stopped, "compaction_boundary"), 1)
		equal(t, len(pp.Requests()), 1)
	})
}
func TestOversizedInputIsNeverSummarized(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := small(t, true, unmeasured("first"), unmeasured("summary one"), tooLong(), unmeasured("summary answer"), tooLong())
		f.done(f.send("one", "t1"))
		huge := strings.Repeat("h", 8000)
		f.fail(f.send(huge, "t2"))
		r := f.p.Requests()
		equal(t, len(r), 5)
		for _, request := range r {
			if isCopy(request) && slices.ContainsFunc(request.Items, func(i provider.InferenceItem) bool { return reflect.DeepEqual(i, userItem(huge)) }) {
				t.Fatal("input summarized")
			}
		}
		equal(t, r[4].Items, []provider.InferenceItem{summaryItem("summary answer"), userItem(huge)})
	})
}
func TestResumePendingSmallerSwitchCompactsOnce(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := setup(t, unmeasured("old answer"), hanging(providertest.Text("partial")), unmeasured("summary"), unmeasured("continued"))
		f.done(f.send(strings.Repeat("x", 3200), "t1"))
		a := f.send("continue", "t2")
		synctest.Wait()
		_, err := f.s.Abort(t.Context())
		must(t, err)
		_, err = a.Wait(t.Context())
		must(t, err)
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: smallModel()}))
		a, err = f.s.Resume()
		must(t, err)
		f.done(a)
		equal(t, summarySizes(f.p), []int{2})
		equal(t, f.p.Requests()[2].ModelID, "test-model")
		equal(t, f.p.Requests()[3].ModelID, "small-model")
		equal(t, countKind(f.s, "resume"), 1)
	})
}
func TestCachedUsageCompactsAfterToolWithoutRerun(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		calls := 0
		r := toolRuntime("work", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			calls++
			return textOutcome("done"), nil
		})
		f := start(t, r, session.DefaultConfig(), providertest.Events(providertest.ToolCall("call", "work", []byte(`{}`)), &provider.Response{Usage: core.TokenUsage{InputTokens: 1, OutputTokens: 1, CacheWriteTokens: 850}}), unmeasured("tool summary"), unmeasured("done"))
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: smallModel()}))
		f.done(f.send("use the tool", "t1"))
		equal(t, calls, 1)
		req := f.p.Requests()
		equal(t, len(req), 3)
		equal(t, req[1].Items, append(slices.Clone(req[0].Items), userItem(sessiontest.CompactionSummaryInstruction)))
		equal(t, req[2].Items[0], summaryItem("tool summary"))
		equal(t, req[2].Items[3], userItem(transcripttest.ResumeText))
	})
}
func TestEmptyOrOnlySummaryCompactionDoesNotRequest(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := small(t, false, unmeasured("first"), unmeasured("summary"), providertest.Events(providertest.Error("failed", nil)), unmeasured("second summary"))
		compact(t, f.s)
		equal(t, len(f.p.Requests()), 0)
		f.done(f.send("one", "t1"))
		compact(t, f.s)
		f.fail(f.send("two", "t2"))
		compact(t, f.s)
		before := f.s.Transcript()
		compact(t, f.s)
		equal(t, len(f.p.Requests()), 4)
		equal(t, f.s.Transcript(), before)
	})
}
func TestContextSourceReannouncesAfterBoundary(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		seen := [][]session.SeenContext{}
		r := &sessiontest.Runtime{News: func(_ context.Context, prior []session.SeenContext, _ core.TurnID) ([]session.NewContext, error) {
			seen = append(seen, slices.Clone(prior))
			if len(prior) == 0 {
				return []session.NewContext{{Source: "execution", Text: "environment"}}, nil
			}
			return nil, nil
		}}
		f := start(t, r, session.DefaultConfig(), unmeasured("one"), unmeasured("two"), unmeasured("summary"), unmeasured("three"))
		f.done(f.send("one", "t1"))
		f.done(f.send("two", "t2"))
		compact(t, f.s)
		f.done(f.send("three", "t3"))
		equal(t, len(seen), 3)
		equal(t, len(seen[0]), 0)
		equal(t, seen[1], []session.SeenContext{{Source: "execution", Text: "environment"}})
		equal(t, len(seen[2]), 0)
		equal(t, countKind(f.s, "context"), 2)
	})
}
func TestSecondSummaryFoldsFirstAndRestores(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := small(t, false, unmeasured("first"), unmeasured("summary one"), unmeasured("second"), unmeasured("summary two"), unmeasured("third"))
		f.done(f.send("one", "t1"))
		compact(t, f.s)
		f.done(f.send("two", "t2"))
		compact(t, f.s)
		cp := f.checkpoint()
		f.done(f.send("three", "t3"))
		r := f.p.Requests()
		equal(t, r[3].Items, []provider.InferenceItem{summaryItem("summary one"), assistantItem("small-model", "first"), userItem("two"), userItem(sessiontest.CompactionSummaryInstruction)})
		equal(t, r[4].Items, []provider.InferenceItem{summaryItem("summary two"), assistantItem("small-model", "second"), userItem("three")})
		s, _, p := f.restore(cp, unmeasured("third"))
		a, err := s.Send(storetest.Text("three"), "t3")
		must(t, err)
		_, err = a.Wait(t.Context())
		must(t, err)
		equal(t, p.Requests()[0].Items, r[4].Items)
	})
}
func TestInputDuringCompactionStaysOutsideSummary(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		turn, entered, release := gated(providertest.Text("summary"), providertest.Response(0, 0))
		nextSummary, nextEntered, nextRelease := gated(providertest.Text("next summary"), providertest.Response(0, 0))
		f := small(t, true, unmeasured("first"), turn, unmeasured("second"), unmeasured("third"), nextSummary, unmeasured("fourth"))
		f.done(f.send(strings.Repeat("x", 3000), "t1"))
		a := f.send(strings.Repeat("y", 400), "t2")
		<-entered
		q := f.send("third", "t3")
		must(t, f.s.Steer(storetest.Text("be brief"), "s1"))
		equal(t, f.s.PendingSteers()[0].TurnID, core.TurnID("t2"))
		close(release)
		f.done(a)
		f.done(q)
		r := f.p.Requests()
		equal(t, steers(r[1]), []string{})
		equal(t, steers(r[2]), []string{"be brief"})
		equal(t, r[3].TurnID, "t3")
		pass, err := f.s.Compact()
		must(t, err)
		<-nextEntered
		queued := f.send("fourth", "t4")
		equal(t, len(f.s.QueuedMessages()), 1)
		close(nextRelease)
		f.done(pass)
		f.done(queued)
		requests := f.p.Requests()
		equal(t, isCopy(requests[4]), true)
		equal(t, requests[5].Items[0], summaryItem("next summary"))
		equal(t, requests[5].TurnID, "t4")
	})
}
func TestEditAroundBoundariesKeepsOnlyPrefixSummaries(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := small(t, false, unmeasured("A"), unmeasured("summary A"), unmeasured("B"), unmeasured("summary B"), unmeasured("C"))
		f.done(f.send("A", "A"))
		compact(t, f.s)
		f.done(f.send("B", "B"))
		compact(t, f.s)
		f.done(f.send("C", "C"))
		cp := f.checkpoint()
		for _, turn := range []core.TurnID{"A", "B", "C"} {
			index := -1
			for i, b := range cp.Transcript {
				if b, ok := b.(*core.UserBlock); ok && b.TurnID == turn {
					index = i
					break
				}
			}
			s, _, p := f.restore(cp, unmeasured("replacement"))
			_, err := s.EditAndSend(t.Context(), edit(t, s, index, "op1"))
			must(t, err)
			must(t, s.Settled(t.Context()))
			equal(t, s.Transcript().Blocks[:index], cp.Transcript[:index])
			req := p.Requests()[0]
			switch turn {
			case "A":
				equal(t, req.Items, []provider.InferenceItem{userItem("replacement")})
			case "B":
				equal(t, req.Items[0], summaryItem("summary A"))
			case "C":
				equal(t, req.Items[0], summaryItem("summary B"))
			}
			must(t, s.Dispose(t.Context()))
		}
	})
}
func TestEditCutMarkerDoesNotReuseOldUsageEstimate(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := small(t, false, unmeasured("A"), providertest.Events(providertest.Text("B"), providertest.Response(900, 0)), providertest.Events(providertest.Error("expired", new(provider.AuthExpired))), unmeasured("summary"))
		f.done(f.send("A", "A"))
		f.done(f.send("B", "B"))
		f.fail(f.send("C", "C"))
		compact(t, f.s)
		cp := f.checkpoint()
		f.config.Compaction = session.DefaultCompactionConfig()
		s, _, p := f.restore(cp, unmeasured("replacement"))
		_, err := s.EditAndSend(t.Context(), edit(t, s, 7, "op1"))
		must(t, err)
		must(t, s.Settled(t.Context()))
		equal(t, len(p.Requests()), 1)
		equal(t, p.Requests()[0].Items, []provider.InferenceItem{summaryItem("summary"), assistantItem("small-model", "B"), userItem("replacement")})
	})
}
func TestPostEditCompactionSummarizesOnlyKeptHistory(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		first := "A " + strings.Repeat("q", 3200)
		f := small(t, false, unmeasured("A"), unmeasured("B"), unmeasured("C"))
		f.done(f.send(first, "A"))
		f.done(f.send("B", "B"))
		f.done(f.send("C", "C"))
		f.config.Compaction = session.DefaultCompactionConfig()
		s, _, p := f.restore(f.checkpoint(), unmeasured("summary A"), unmeasured("replacement"))
		_, err := s.EditAndSend(t.Context(), edit(t, s, 3, "op1"))
		must(t, err)
		must(t, s.Settled(t.Context()))
		equal(t, p.Requests()[0].Items, []provider.InferenceItem{userItem(first), userItem(sessiontest.CompactionSummaryInstruction)})
		equal(t, p.Requests()[1].Items, []provider.InferenceItem{summaryItem("summary A"), assistantItem("small-model", "A"), userItem("replacement")})
	})
}
func TestScreenshotsCompactBeforeImageLimit(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r := toolRuntime("shoot", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			return session.ToolOutcome{Output: []provider.ResultPart{&provider.ResultImage{Bytes: provider.MediaBytes{Data: storetest.PNG(3, 2, 1), MediaType: "image/png"}}}}, nil
		})
		f := start(t, r, session.DefaultConfig(), tool("shoot"), tool("shoot"), tool("shoot"), tool("shoot"), unmeasured("screen so far"), unmeasured("done"))
		f.p.SetLimits(provider.RequestLimits{Images: new(uint32(5))})
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: storetest.ModelReading("stub", "test-model", []core.FileExtension{core.FileExtensionPNG})}))
		f.done(f.send("watch screen", "t1"))
		requests := f.p.Requests()
		counts := []uint64{}
		for _, req := range requests {
			counts = append(counts, transcript.MeasureRequest("", req.Items).Images)
		}
		equal(t, counts, []uint64{0, 1, 2, 3, 3, 1})
		equal(t, requests[4].Items, append(slices.Clone(requests[3].Items), userItem(sessiontest.CompactionSummaryInstruction)))
		equal(t, requests[5].Items[3], userItem(transcripttest.ResumeText))
	})
}
func TestContextRefusalCompactsOnceThenFails(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := small(t, true, unmeasured("first"), tooLong(), unmeasured("summary"), unmeasured("second"), tooLong(), unmeasured("second summary"), tooLong())
		f.done(f.send("one", "t1"))
		retries := 0
		sub := f.s.Subscribe(func(e session.Event) {
			if _, ok := e.(*session.RetryScheduled); ok {
				retries++
			}
		})
		defer sub.Release()
		f.done(f.send("two", "t2"))
		equal(t, f.p.Requests()[3].Items, []provider.InferenceItem{summaryItem("summary"), assistantItem("small-model", "first"), userItem("two")})
		f.fail(f.send("three", "t3"))
		equal(t, retries, 0)
		equal(t, f.p.Remaining(), 0)
		equal(t, countKind(f.s, "error"), 1)
	})
}
func TestSwitchToFewerImagesCompactsWithOldProvider(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r := toolRuntime("shoot", func(context.Context, session.ToolInvocation) (session.ToolOutcome, error) {
			return session.ToolOutcome{Output: []provider.ResultPart{&provider.ResultImage{Bytes: provider.MediaBytes{Data: storetest.PNG(3, 2, 1), MediaType: "image/png"}}}}, nil
		})
		f := start(t, r, session.DefaultConfig(), tool("shoot"), tool("shoot"), tool("shoot"), unmeasured("done"), unmeasured("summary"))
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: storetest.ModelReading("stub", "model-a", []core.FileExtension{core.FileExtensionPNG})}))
		f.done(f.send("watch", "t1"))
		next := providertest.NewScriptedRuntime(t, unmeasured("next"))
		next.SetLimits(provider.RequestLimits{Images: new(uint32(4))})
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: storetest.ModelReading("other", "model-b", []core.FileExtension{core.FileExtensionPNG}), Runtime: next}))
		f.done(f.send("continue", "t2"))
		old := f.p.Requests()
		equal(t, isCopy(old[len(old)-1]), true)
		equal(t, old[len(old)-1].ModelID, "model-a")
		equal(t, transcript.MeasureRequest("", next.Requests()[0].Items).Images, uint64(0))
	})
}
