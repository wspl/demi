package transcript_test

import (
	"errors"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
)

func TestResumeDropsOnlyUnactedLeftovers(t *testing.T) {
	thinking := &core.ThinkingBlock{Text: "hmm"}
	failure := &core.ErrorBlock{Message: "failed"}
	user := userBlock("u1", "")
	for _, tc := range []struct {
		name   string
		blocks []core.Block
		want   transcript.ResumePoint
	}{
		{"failed", []core.Block{user, thinking, failure}, transcript.ResumePoint{Cut: 1, FullRerun: true}},
		{"blank", []core.Block{user, textBlock("text", " \n"), failure}, transcript.ResumePoint{Cut: 1, FullRerun: true}},
		{"posted", []core.Block{user, textBlock("text", "posted"), failure}, transcript.ResumePoint{Cut: 2}},
		{"response", []core.Block{user, responseBlock("r", 0), thinking}, transcript.ResumePoint{Cut: 2}},
		{"stop", []core.Block{user, &core.AbortBlock{}}, transcript.ResumePoint{Cut: 2}},
		{"completed call", []core.Block{user, &core.ToolCallBlock{Status: "completed"}, thinking, failure}, transcript.ResumePoint{Cut: 2}},
		{"executing call", []core.Block{user, &core.ToolCallBlock{Status: "executing"}, thinking, failure}, transcript.ResumePoint{Cut: 2}},
		{"latest turn", []core.Block{user, textBlock("a", "answer"), responseBlock("r", 0), userBlock("u2", ""), failure}, transcript.ResumePoint{Cut: 4, FullRerun: true}},
		{"empty", nil, transcript.ResumePoint{}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := transcript.Cut(tc.blocks); got != tc.want {
				t.Fatalf("got %+v, want %+v", got, tc.want)
			}
		})
	}
}

func TestRewindAndEditForkBoundaries(t *testing.T) {
	user := userBlock("u", "hello")
	steer := &core.SteerBlock{BlockID: "s", TurnID: "u"}
	message := &core.AgentMessageBlock{BlockID: "m", TurnID: "u"}
	answer := &core.TextBlock{BlockID: "a", Text: "done", Forkable: true}
	blocks := []core.Block{user, &core.ThinkingBlock{}, steer, answer, message}
	want := &transcript.Rewound{Retained: []core.Block{user, steer, message}, Input: 0, Turn: "u"}
	if got := transcript.Rewind(blocks); !reflect.DeepEqual(want, got) {
		t.Fatalf("got %+v, want %+v", got, want)
	}
	continuation := &core.AgentMessageBlock{BlockID: "next", TurnID: "next"}
	blocks = append(blocks, continuation, textBlock("later", "continued"))
	got := transcript.Rewind(blocks)
	if got.Input != 5 || got.Turn != "next" || len(got.Retained) != 6 {
		t.Fatalf("continuation rewind: %+v", got)
	}
	if transcript.Rewind([]core.Block{answer}) != nil {
		t.Fatal("rewound without input")
	}
	if got := transcript.LastAssistantText(blocks, 4); got != "continued" {
		t.Fatal(got)
	}
	if got := transcript.LastAssistantText(blocks, 99); got != "" {
		t.Fatal(got)
	}
	if prefix, err := transcript.BeforeUser(blocks, "u"); err != nil || len(prefix) != 0 {
		t.Fatalf("%v %v", prefix, err)
	}
	if _, err := transcript.BeforeUser(blocks, "a"); !errors.Is(err, transcript.NotUserMessage) {
		t.Fatal(err)
	}
	if prefix, err := transcript.ThroughAssistant(blocks, "a"); err != nil || len(prefix) != 4 {
		t.Fatalf("%v %v", prefix, err)
	}
	if _, err := transcript.ThroughAssistant(blocks, "later"); !errors.Is(err, transcript.NotCompletedText) {
		t.Fatal(err)
	}
	blocks[1] = &core.ToolCallBlock{Status: "executing"}
	if _, err := transcript.ThroughAssistant(blocks, "a"); !errors.Is(err, transcript.UnfinishedToolCalls) {
		t.Fatal(err)
	}
	for _, b := range []core.Block{&core.ContextBlock{}, &core.WakeupBlock{Placement: "new_turn"}} {
		if !transcript.Cut([]core.Block{b, &core.RedactedThinkingBlock{}}).FullRerun {
			t.Fatalf("not input: %T", b)
		}
	}
	if transcript.OpensInputTurn(&core.WakeupBlock{Placement: "steer"}) {
		t.Fatal("steer opened turn")
	}
}

func TestCompactionWindowKeepsUnansweredInput(t *testing.T) {
	user := userBlock("u", "input")
	boundary := &core.CompactionBoundaryBlock{BlockID: "b"}
	marker := &core.CompactionMarkerBlock{BoundaryID: "b"}
	for _, tc := range []struct {
		name   string
		blocks []core.Block
		want   transcript.CompactionWindow
	}{
		{"answered", []core.Block{user, textBlock("a", "answer"), responseBlock("r", 1)}, transcript.CompactionWindow{Cut: 1}},
		{"unanswered", []core.Block{user, textBlock("a", "partial"), &core.ResumeBlock{}}, transcript.CompactionWindow{}},
		{"response then input", []core.Block{responseBlock("r", 1), marker, user, &core.ErrorBlock{}}, transcript.CompactionWindow{Cut: 2}},
		{"after compaction", []core.Block{user, boundary, textBlock("a", "kept"), responseBlock("r", 1), marker}, transcript.CompactionWindow{Start: 1, Cut: 4}},
		{"boundary without marker", []core.Block{user, boundary, textBlock("a", "kept"), responseBlock("r", 1)}, transcript.CompactionWindow{Start: 1, Cut: 2}},
		{"never below boundary", []core.Block{user, boundary, marker}, transcript.CompactionWindow{Start: 1, Cut: 1}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := transcript.Window(tc.blocks); got != tc.want {
				t.Fatalf("%+v != %+v", got, tc.want)
			}
		})
	}
}
