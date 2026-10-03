package transcript

import (
	"slices"
	"time"

	"github.com/wspl/demi/internal/core"
)

// Kept is how long a tool result's image or video stays in its block.
const Kept = 30 * 24 * time.Hour

// CacheLifetime is the longest time a vendor keeps a request in its cache.
const CacheLifetime = 24 * time.Hour

// Retirement identifies when the rule is applied and whether the conversation
// has been idle for Kept, permitting every expired tool medium to retire.
type Retirement struct {
	// Now is the time against which retention is checked.
	Now core.Timestamp
	// Idle reports whether the conversation is idle.
	Idle bool
}

// RetiredBlock replaces the transcript block at Index with Value.
type RetiredBlock struct {
	// Index is the transcript row to replace.
	Index int
	// Value contains the block with retired media records.
	Value core.Block
}

// Retire returns replacements for expired tool images and videos, without
// changing blocks. A tool call must be older than Kept and either the conversation
// is idle or the call precedes a boundary older than CacheLifetime.
func Retire(blocks []core.Block, retirement Retirement) []RetiredBlock {
	start := ReplayStart(blocks)
	summarized := false
	if start < len(blocks) {
		if boundary, ok := blocks[start].(*core.CompactionBoundaryBlock); ok {
			summarized = olderThan(boundary.Timestamp, CacheLifetime, retirement.Now)
		}
	}
	changed := []RetiredBlock{}
	for i, block := range blocks {
		call, ok := block.(*core.ToolCallBlock)
		if !ok {
			continue
		}
		eligible := retirement.Idle || (summarized && i < start)
		if !eligible || !olderThan(call.Timestamp, Kept, retirement.Now) {
			continue
		}
		next := *call
		next.Output = slices.Clone(call.Output)
		retired := false
		for j, part := range next.Output {
			var kind core.ModelMediaKind
			var source core.ToolMediaSource
			switch p := part.(type) {
			case *core.ToolImage:
				kind = "image"
				source = p.Source
			case *core.ToolVideo:
				kind = "video"
				source = p.Source
			case *core.ToolText, *core.ToolGone:
				continue
			}
			switch s := source.(type) {
			case *core.ToolMediaRef:
				next.Output[j] = &core.ToolGone{
					Kind:      kind,
					MediaType: s.MediaType,
					Cause:     &core.Retired{At: retirement.Now},
				}
				retired = true
			}
		}
		if retired {
			changed = append(changed, RetiredBlock{Index: i, Value: &next})
		}
	}
	return changed
}

// olderThan compares validated transcript timestamps against a retention age.
func olderThan(timestamp core.Timestamp, age time.Duration, now core.Timestamp) bool {
	// Both timestamps are validated at entry; their conversion cannot fail.
	before, _ := timestamp.Time()
	after, _ := now.Time()
	return after.Sub(before) > age
}
