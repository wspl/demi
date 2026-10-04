package transcript_test

import (
	"reflect"
	"testing"
	"time"

	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/types"
)

func TestToolMediaRetirementAfterCacheExpiry(t *testing.T) {
	now := types.Timestamp("2026-10-01T12:00:00.000Z")
	daysAgo := func(days int) types.Timestamp {
		stamp, err := types.TimestampFromTime(
			time.Date(2026, 10, 1, 12, 0, 0, 0, time.UTC).Add(-time.Duration(days) * 24 * time.Hour),
		)
		if err != nil {
			t.Fatal(err)
		}
		return stamp
	}
	shot := func(name string, days int) types.Block {
		return &types.ToolCallBlock{
			BlockID:   types.BlockID(name),
			Timestamp: daysAgo(days),
			Selection: storetest.TestModel(),
			ToolUseID: "toolu_" + name,
			ToolName:  "shell_exec",
			Input:     "{}",
			Status:    "completed",
			Output: []types.ToolResultContentBlock{
				&types.ToolText{Text: "exit 0"},
				&types.ToolImage{Source: &types.ToolMediaRef{Ref: types.BlobRefOf([]byte{1}), MediaType: "image/png"}},
				&types.ToolVideo{Source: &types.ToolMediaRef{Ref: types.BlobRefOf([]byte{2}), MediaType: "video/mp4"}},
			},
		}
	}
	upload := func(days int) types.Block {
		return &types.UserBlock{
			BlockID:   "u",
			TurnID:    "u",
			Timestamp: daysAgo(days),
			Selection: storetest.TestModel(),
			Content: []types.UserContentBlock{
				&types.UserImage{
					Source: &types.MediaSourceRef{Ref: types.BlobRefOf([]byte{3}), MediaType: "image/png"},
				},
			},
		}
	}
	boundary := func(days int) types.Block {
		return &types.CompactionBoundaryBlock{
			BlockID:       "b",
			Timestamp:     daysAgo(days),
			Selection:     storetest.TestModel(),
			Summary:       "Earlier work.",
			SummaryTokens: 3,
		}
	}
	for _, scenario := range []struct {
		name    string
		blocks  []types.Block
		idle    bool
		indices []int
	}{
		{"before old boundary", []types.Block{upload(40), shot("a", 31), boundary(2), shot("b", 31)}, false, []int{1}},
		{"exactly thirty days", []types.Block{shot("a", 30), boundary(2)}, false, []int{}},
		{"recent boundary", []types.Block{shot("a", 31), boundary(0)}, false, []int{}},
		{"exact cache lifetime", []types.Block{shot("a", 31), boundary(1)}, false, []int{}},
		{"active replay", []types.Block{shot("a", 31)}, false, []int{}},
		{"idle", []types.Block{shot("a", 31), boundary(0), shot("b", 31), shot("c", 29)}, true, []int{0, 2}},
		{"uploads never retire", []types.Block{upload(90), boundary(40)}, true, []int{}},
		{"only last boundary", []types.Block{shot("a", 31), boundary(5), shot("b", 31), boundary(0)}, false, []int{}},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			changed := transcript.Retire(scenario.blocks, transcript.Retirement{Now: now, Idle: scenario.idle})
			indices := make([]int, 0, len(changed))
			for _, change := range changed {
				indices = append(indices, change.Index)
				original := scenario.blocks[change.Index].(*types.ToolCallBlock)
				want := *original
				want.Output = []types.ToolResultContentBlock{
					&types.ToolText{Text: "exit 0"},
					&types.ToolGone{Kind: "image", MediaType: "image/png", Cause: &types.Retired{At: now}},
					&types.ToolGone{Kind: "video", MediaType: "video/mp4", Cause: &types.Retired{At: now}},
				}
				if !reflect.DeepEqual(change.Value, &want) {
					t.Fatalf("got %+v want %+v", change.Value, &want)
				}
				if _, ok := original.Output[1].(*types.ToolImage); !ok {
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
