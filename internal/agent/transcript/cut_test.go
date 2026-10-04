package transcript_test

import (
	"errors"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/types"
)

func TestResumeDropsOnlyUnactedLeftovers(t *testing.T) {
	thinking := &types.ThinkingBlock{Text: "hmm"}
	failure := &types.ErrorBlock{Message: "failed"}
	user := userBlock("u1", "")
	for _, scenario := range []struct {
		name   string
		blocks []types.Block
		want   transcript.ResumePoint
	}{
		{"failed", []types.Block{user, thinking, failure}, transcript.ResumePoint{Cut: 1, FullRerun: true}},
		{"blank", []types.Block{user, textBlock("text", " \n"), failure}, transcript.ResumePoint{Cut: 1, FullRerun: true}},
		{"posted", []types.Block{user, textBlock("text", "posted"), failure}, transcript.ResumePoint{Cut: 2}},
		{"response", []types.Block{user, responseBlock("r", 0), thinking}, transcript.ResumePoint{Cut: 2}},
		{"stop", []types.Block{user, &types.AbortBlock{}}, transcript.ResumePoint{Cut: 2}},
		{
			"completed call",
			[]types.Block{
				user,
				&types.ToolCallBlock{Status: "completed"},
				thinking,
				failure,
			},
			transcript.ResumePoint{Cut: 2},
		},
		{
			"executing call",
			[]types.Block{
				user,
				&types.ToolCallBlock{Status: "executing"},
				thinking,
				failure,
			},
			transcript.ResumePoint{Cut: 2},
		},
		{
			"latest turn",
			[]types.Block{
				user,
				textBlock("a", "answer"),
				responseBlock("r", 0),
				userBlock("u2", ""),
				failure,
			},
			transcript.ResumePoint{
				Cut:       4,
				FullRerun: true,
			},
		},
		{"empty", nil, transcript.ResumePoint{}},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			if got := transcript.Cut(scenario.blocks); got != scenario.want {
				t.Fatalf("got %+v, want %+v", got, scenario.want)
			}
		})
	}
}

func TestRewindAndEditForkBoundaries(t *testing.T) {
	user := userBlock("u", "hello")
	steer := &types.SteerBlock{BlockID: "s", TurnID: "u"}
	message := &types.AgentMessageBlock{BlockID: "m", TurnID: "u"}
	answer := &types.TextBlock{BlockID: "a", Text: "done", Forkable: true}
	blocks := []types.Block{user, &types.ThinkingBlock{}, steer, answer, message}
	want := transcript.Rewound{Retained: []types.Block{user, steer, message}, Input: 0, Turn: "u"}
	if got, ok := transcript.Rewind(blocks); !ok || !reflect.DeepEqual(want, got) {
		t.Fatalf("got %+v, want %+v", got, want)
	}
	continuation := &types.AgentMessageBlock{BlockID: "next", TurnID: "next"}
	blocks = append(blocks, continuation, textBlock("later", "continued"))
	got, _ := transcript.Rewind(blocks)
	if got.Input != 5 || got.Turn != "next" || len(got.Retained) != 6 {
		t.Fatalf("continuation rewind: %+v", got)
	}
	if _, ok := transcript.Rewind([]types.Block{answer}); ok {
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
	if _, err := transcript.BeforeUser(blocks, "a"); !errors.Is(err, transcript.ErrNotUserMessage) {
		t.Fatal(err)
	}
	if prefix, err := transcript.ThroughAssistant(blocks, "a"); err != nil || len(prefix) != 4 {
		t.Fatalf("%v %v", prefix, err)
	}
	if _, err := transcript.ThroughAssistant(blocks, "later"); !errors.Is(err, transcript.ErrNotCompletedText) {
		t.Fatal(err)
	}
	blocks[1] = &types.ToolCallBlock{Status: "executing"}
	if _, err := transcript.ThroughAssistant(blocks, "a"); !errors.Is(err, transcript.ErrUnfinishedToolCalls) {
		t.Fatal(err)
	}
	for _, b := range []types.Block{&types.ContextBlock{}, &types.WakeupBlock{Placement: "new_turn"}} {
		if !transcript.Cut([]types.Block{b, &types.RedactedThinkingBlock{}}).FullRerun {
			t.Fatalf("not input: %T", b)
		}
	}
	if transcript.OpensInputTurn(&types.WakeupBlock{Placement: "steer"}) {
		t.Fatal("steer opened turn")
	}
}

func TestCompactionWindowKeepsUnansweredInput(t *testing.T) {
	user := userBlock("u", "input")
	boundary := &types.CompactionBoundaryBlock{BlockID: "b"}
	marker := &types.CompactionMarkerBlock{BoundaryID: "b"}
	for _, scenario := range []struct {
		name   string
		blocks []types.Block
		want   transcript.CompactionWindow
	}{
		{
			"answered",
			[]types.Block{
				user,
				textBlock("a", "answer"),
				responseBlock("r", 1),
			},
			transcript.CompactionWindow{Cut: 1},
		},
		{"unanswered", []types.Block{user, textBlock("a", "partial"), &types.ResumeBlock{}}, transcript.CompactionWindow{}},
		{
			"response then input",
			[]types.Block{
				responseBlock("r", 1),
				marker,
				user,
				&types.ErrorBlock{},
			},
			transcript.CompactionWindow{Cut: 2},
		},
		{
			"after compaction",
			[]types.Block{
				user,
				boundary,
				textBlock("a", "kept"),
				responseBlock("r", 1),
				marker,
			},
			transcript.CompactionWindow{
				Start: 1,
				Cut:   4,
			},
		},
		{
			"boundary without marker",
			[]types.Block{
				user,
				boundary,
				textBlock("a", "kept"),
				responseBlock("r", 1),
			},
			transcript.CompactionWindow{
				Start: 1,
				Cut:   2,
			},
		},
		{"never below boundary", []types.Block{user, boundary, marker}, transcript.CompactionWindow{Start: 1, Cut: 1}},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			if got := transcript.Window(scenario.blocks); got != scenario.want {
				t.Fatalf("%+v != %+v", got, scenario.want)
			}
		})
	}
}
