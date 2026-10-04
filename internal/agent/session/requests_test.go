package session_test

import (
	"context"
	"slices"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/session/sessiontest"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
)

// TestRequestPrefixesWithThinkingAndSummary checks the requests a session
// builds. The vendor wire forms of the seven provider families are tested in
// internal/backend/providerhost.
func TestRequestPrefixesWithThinkingAndSummary(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		entered, release := make(chan struct{}), make(chan struct{})
		r := &sessiontest.Runtime{
			Prompt: "system",
			Definitions: []provider.ToolDefinition{
				{Name: "look", InputSchema: []byte(`{"type":"object"}`)},
				{Name: "note", InputSchema: []byte(`{"type":"object"}`)},
				{Name: "yield", InputSchema: []byte(`{"type":"object"}`)},
			},
		}
		r.Invoke = func(ctx context.Context, call session.ToolInvocation) (session.ToolOutcome, error) {
			switch call.ToolName {
			case "look":
				close(entered)
				select {
				case <-release:
					return session.ToolOutcome{
						Output: []provider.ResultPart{
							&provider.ResultImage{
								Bytes: provider.MediaBytes{Data: storetest.PNG(3, 2, 2), MediaType: "image/png"},
							},
						},
					}, nil
				case <-ctx.Done():
					return session.ToolOutcome{}, ctx.Err()
				}
			case "yield":
				return session.ToolOutcome{Effect: &session.ScheduleYield{DurationMS: 600000}}, nil
			default:
				return textOutcome("noted"), nil
			}
		}
		f := start(
			t,
			r,
			session.DefaultConfig(),
			providertest.Events(
				&provider.ThinkingStart{},
				providertest.Thinking("first"),
				&provider.ThinkingSignature{Signature: "anthropic:first"},
				providertest.Text("Looking."),
				providertest.ToolCall("look", "look", []byte(`{}`)),
				providertest.ToolCall("note", "note", []byte(`{}`)),
				providertest.Response(1, 1),
			),
			answer("Both seen."),
			providertest.Events(
				&provider.ThinkingStart{},
				providertest.Thinking("second"),
				&provider.ThinkingSignature{Signature: "anthropic:second"},
				providertest.ToolCall("yield", "yield", []byte(`{}`)),
				providertest.Response(1, 1),
			),
			answer("The user showed a screenshot and asked twice."),
			answer("After summary."),
			answer("Thought harder."),
		)
		model := storetest.ModelReading("stub", "model-a", []types.FileExtension{types.FileExtensionPNG})
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: model}))
		png := storetest.PNG(4, 3, 1)
		held := store.HeldMedia{}
		held.Hold(types.BlobRefOf(png), png)
		f.s.HoldMedia(&held)
		a, err := f.s.Send(
			[]types.UserContentBlock{
				&types.UserText{Text: "What is on screen?"},
				&types.UserImage{Source: &types.MediaSourceRef{Ref: types.BlobRefOf(png), MediaType: "image/png"}},
			},
			"t1",
		)
		must(t, err)
		<-entered
		must(t, f.s.Steer(storetest.Text("mind tests"), "s1"))
		must(t, f.s.AcceptAgentMessage(t.Context(), message("m1")))
		close(release)
		f.done(a)
		f.done(f.send("And the rest?", "t2"))
		compact(t, f.s)
		f.done(f.send("Go on.", "t3"))
		model.Thinking = &types.AdaptiveConfig{Effort: "high"}
		must(t, f.s.UpdateModel(session.ModelSwitch{Model: model}))
		f.done(f.send("Think harder.", "t4"))
		req := f.p.Requests()
		equal(t, len(req), 6)
		for i := 1; i <= 2; i++ {
			equal(t, req[i].Items[:len(req[i-1].Items)], req[i-1].Items)
		}
		equal(t, req[3].Items, append(slices.Clone(req[2].Items), userItem(sessiontest.CompactionSummaryInstruction)))
		equal(t, req[4].Items[0], summaryItem("The user showed a screenshot and asked twice."))
		equal(t, req[5].Items[:len(req[4].Items)], req[4].Items)
		equal(t, req[4].Thinking, types.ThinkingConfig(nil))
		equal(t, req[5].Thinking, model.Thinking)
		for _, request := range req {
			equal(t, request.SystemPrompt, "system")
			equal(t, request.Tools, r.Definitions)
		}
	})
}
