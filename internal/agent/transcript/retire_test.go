package transcript_test

import (
	"reflect"
	"testing"
	"time"

	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
)

func TestToolMediaRetirementAfterCacheExpiry(t *testing.T) {
	now := core.Timestamp("2026-10-01T12:00:00.000Z")
	daysAgo := func(days int) core.Timestamp {
		stamp, err := core.TimestampFromTime(
			time.Date(2026, 10, 1, 12, 0, 0, 0, time.UTC).Add(-time.Duration(days) * 24 * time.Hour),
		)
		if err != nil {
			t.Fatal(err)
		}
		return stamp
	}
	shot := func(name string, days int) core.Block {
		return &core.ToolCallBlock{
			BlockID:   core.BlockID(name),
			Timestamp: daysAgo(days),
			Selection: storetest.TestModel(),
			ToolUseID: "toolu_" + name,
			ToolName:  "shell_exec",
			Input:     "{}",
			Status:    "completed",
			Output: []core.ToolResultContentBlock{
				&core.ToolText{Text: "exit 0"},
				&core.ToolImage{Source: &core.ToolMediaRef{Ref: core.BlobRefOf([]byte{1}), MediaType: "image/png"}},
				&core.ToolVideo{Source: &core.ToolMediaRef{Ref: core.BlobRefOf([]byte{2}), MediaType: "video/mp4"}},
			},
		}
	}
	upload := func(days int) core.Block {
		return &core.UserBlock{
			BlockID:   "u",
			TurnID:    "u",
			Timestamp: daysAgo(days),
			Selection: storetest.TestModel(),
			Content: []core.UserContentBlock{
				&core.UserImage{Source: &core.MediaSourceRef{Ref: core.BlobRefOf([]byte{3}), MediaType: "image/png"}},
			},
		}
	}
	boundary := func(days int) core.Block {
		return &core.CompactionBoundaryBlock{
			BlockID:       "b",
			Timestamp:     daysAgo(days),
			Selection:     storetest.TestModel(),
			Summary:       "Earlier work.",
			SummaryTokens: 3,
		}
	}
	for _, scenario := range []struct {
		name    string
		blocks  []core.Block
		idle    bool
		indices []int
	}{
		{"before old boundary", []core.Block{upload(40), shot("a", 31), boundary(2), shot("b", 31)}, false, []int{1}},
		{"exactly thirty days", []core.Block{shot("a", 30), boundary(2)}, false, []int{}},
		{"recent boundary", []core.Block{shot("a", 31), boundary(0)}, false, []int{}},
		{"exact cache lifetime", []core.Block{shot("a", 31), boundary(1)}, false, []int{}},
		{"active replay", []core.Block{shot("a", 31)}, false, []int{}},
		{"idle", []core.Block{shot("a", 31), boundary(0), shot("b", 31), shot("c", 29)}, true, []int{0, 2}},
		{"uploads never retire", []core.Block{upload(90), boundary(40)}, true, []int{}},
		{"only last boundary", []core.Block{shot("a", 31), boundary(5), shot("b", 31), boundary(0)}, false, []int{}},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			changed := transcript.Retire(scenario.blocks, transcript.Retirement{Now: now, Idle: scenario.idle})
			indices := make([]int, 0, len(changed))
			for _, change := range changed {
				indices = append(indices, change.Index)
				original := scenario.blocks[change.Index].(*core.ToolCallBlock)
				want := *original
				want.Output = []core.ToolResultContentBlock{
					&core.ToolText{Text: "exit 0"},
					&core.ToolGone{Kind: "image", MediaType: "image/png", Cause: &core.Retired{At: now}},
					&core.ToolGone{Kind: "video", MediaType: "video/mp4", Cause: &core.Retired{At: now}},
				}
				if !reflect.DeepEqual(change.Value, &want) {
					t.Fatalf("got %+v want %+v", change.Value, &want)
				}
				if _, ok := original.Output[1].(*core.ToolImage); !ok {
					t.Fatal("mutated original")
				}
				scenario.blocks[change.Index] = change.Value
			}
			if !reflect.DeepEqual(indices, scenario.indices) {
				t.Fatalf("%v != %v", indices, scenario.indices)
			}
			if next := transcript.Retire(
				scenario.blocks,
				transcript.Retirement{Now: now, Idle: scenario.idle},
			); len(next) != 0 {
				t.Fatalf("retired twice: %+v", next)
			}
		})
	}
}
