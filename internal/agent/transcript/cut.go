package transcript

import (
	"errors"
	"slices"

	"github.com/wspl/demi/internal/types"
)

// ResumePoint identifies where re-inference restarts after an unfinished turn.
type ResumePoint struct {
	// Cut is the first index of the unfinished attempt's leftovers.
	Cut int
	// FullRerun says the whole attempt was leftovers, so resume reruns like retry.
	FullRerun bool
}

// Cut scans back over reasoning, errors, and blank text that nobody acted on.
// Reaching the input that opened the turn makes the cut a full rerun.
func Cut(blocks []types.Block) ResumePoint {
	for i := len(blocks) - 1; i >= 0; i-- {
		if OpensInputTurn(blocks[i]) {
			return ResumePoint{Cut: i + 1, FullRerun: true}
		}
		leftover := false
		switch b := blocks[i].(type) {
		case *types.ThinkingBlock, *types.RedactedThinkingBlock, *types.ErrorBlock:
			leftover = true
		case *types.TextBlock:
			leftover = types.IsBlank(b.Text)
		case *types.UserBlock,
			*types.ContextBlock,
			*types.WakeupBlock,
			*types.SteerBlock,
			*types.AgentMessageBlock,
			*types.ResumeBlock,
			*types.AbortBlock,
			*types.ToolCallBlock,
			*types.ResponseBlock,
			*types.CompactionBoundaryBlock,
			*types.CompactionMarkerBlock:
		}
		if !leftover {
			return ResumePoint{Cut: i + 1}
		}
	}
	return ResumePoint{Cut: len(blocks)}
}

// Rewound is the history retry keeps, including the turn's steers and all later
// agent messages.
type Rewound struct {
	// Retained contains the blocks preserved for retry.
	Retained []types.Block
	// Input is the index of the opening input in Retained.
	Input int
	// Turn is the input's turn, which the rerun continues.
	Turn types.TurnID
}

// Rewind retains the last input turn for retry, and returns false without one.
// The first agent message of a continuation also opens its turn.
func Rewind(blocks []types.Block) (Rewound, bool) {
	seen := map[types.TurnID]bool{}
	start := -1
	for i, b := range blocks {
		turn := turnOf(b)
		if turn == "" {
			continue
		}
		_, message := b.(*types.AgentMessageBlock)
		if OpensInputTurn(b) || (message && !seen[turn]) {
			start = i
		}
		seen[turn] = true
	}
	if start < 0 {
		return Rewound{}, false
	}
	turn := turnOf(blocks[start])
	retained := slices.Clone(blocks[:start+1])
	for _, b := range blocks[start+1:] {
		if _, ok := b.(*types.AgentMessageBlock); ok {
			retained = append(retained, b)
		}
		if steer, ok := b.(*types.SteerBlock); ok && steer.TurnID == turn {
			retained = append(retained, b)
		}
	}
	return Rewound{Retained: retained, Input: start, Turn: turn}, true
}

// turnOf identifies the turn to which a transcript input belongs.
func turnOf(block types.Block) types.TurnID {
	switch b := block.(type) {
	case *types.UserBlock:
		return b.TurnID
	case *types.ContextBlock:
		return b.TurnID
	case *types.WakeupBlock:
		return b.TurnID
	case *types.SteerBlock:
		return b.TurnID
	case *types.AgentMessageBlock:
		return b.TurnID
	case *types.ResumeBlock:
		return b.TurnID
	case *types.ThinkingBlock,
		*types.RedactedThinkingBlock,
		*types.TextBlock,
		*types.ErrorBlock,
		*types.AbortBlock,
		*types.ToolCallBlock,
		*types.ResponseBlock,
		*types.CompactionBoundaryBlock,
		*types.CompactionMarkerBlock:
		return ""
	}
	return ""
}

//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
var (
	// ErrNotUserMessage rejects an edit target that is not a user message.
	ErrNotUserMessage = errors.New("The edit target must be a user message")
	// ErrNotCompletedText rejects a Fork target that is not completed answer text.
	ErrNotCompletedText = errors.New("The Fork target must be a completed assistant message")
	// ErrUnfinishedToolCalls rejects a Fork prefix with an executing tool call.
	ErrUnfinishedToolCalls = errors.New("The Fork boundary contains unfinished tool calls")
)

// BeforeUser returns the blocks before the editable user target.
func BeforeUser(blocks []types.Block, target types.BlockID) ([]types.Block, error) {
	for i, b := range blocks {
		if b.ID() != target {
			continue
		}
		if !b.IsEditable() {
			break
		}
		return slices.Clone(blocks[:i]), nil
	}
	return nil, ErrNotUserMessage
}

// ThroughAssistant returns the blocks through a completed answer target,
// refusing a prefix with unfinished tool calls.
func ThroughAssistant(blocks []types.Block, target types.BlockID) ([]types.Block, error) {
	for i, b := range blocks {
		if b.ID() != target {
			continue
		}
		text, ok := b.(*types.TextBlock)
		if !ok || !text.Forkable {
			break
		}
		for _, prior := range blocks[:i+1] {
			if call, ok := prior.(*types.ToolCallBlock); ok && call.Status == "executing" {
				return nil, ErrUnfinishedToolCalls
			}
		}
		return slices.Clone(blocks[:i+1]), nil
	}
	return nil, ErrNotCompletedText
}

// CompactionWindow is the half-open range of blocks the next pass summarizes.
type CompactionWindow struct {
	// Start is the first block included in the summary.
	Start int
	// Cut is the first block after the summarized range.
	Cut int
}

// Window ends at the latest answered request's answer, or at unanswered input
// when no request has been answered since the last compaction.
func Window(blocks []types.Block) CompactionWindow {
	start := ReplayStart(blocks)
	cut := latestAnswer(blocks)
	if cut < 0 {
		answered := 0
		for i := len(blocks) - 1; i >= 0; i-- {
			if _, ok := blocks[i].(*types.ResponseBlock); ok {
				answered = i + 1
				break
			}
		}
		cut = answered
		for i := len(blocks) - 1; i >= answered; i-- {
			if _, ok := blocks[i].(*types.UserBlock); ok {
				cut = i
				break
			}
		}
		cut = max(cut, start)
	}
	return CompactionWindow{Start: start, Cut: cut}
}

// LastAssistantText returns the last answer text from since onward, or empty.
func LastAssistantText(blocks []types.Block, since int) string {
	for i := len(blocks) - 1; i >= max(since, 0); i-- {
		if b, ok := blocks[i].(*types.TextBlock); ok {
			return b.Text
		}
	}
	return ""
}

// ReplayStart returns the last compaction boundary's index, or zero.
func ReplayStart(blocks []types.Block) int {
	for i := len(blocks) - 1; i >= 0; i-- {
		if _, ok := blocks[i].(*types.CompactionBoundaryBlock); ok {
			return i
		}
	}
	return 0
}

// OpensInputTurn reports whether a block opens an input turn for recovery and
// before-user command-state boundaries.
func OpensInputTurn(block types.Block) bool {
	switch b := block.(type) {
	case *types.UserBlock, *types.ContextBlock:
		return true
	case *types.WakeupBlock:
		return b.Placement == "new_turn"
	case *types.SteerBlock,
		*types.AgentMessageBlock,
		*types.ResumeBlock,
		*types.ThinkingBlock,
		*types.RedactedThinkingBlock,
		*types.TextBlock,
		*types.ErrorBlock,
		*types.AbortBlock,
		*types.ToolCallBlock,
		*types.ResponseBlock,
		*types.CompactionBoundaryBlock,
		*types.CompactionMarkerBlock:
		return false
	}
	return false
}

// latestAnswer locates the answer to the latest request after compaction, or -1.
func latestAnswer(blocks []types.Block) int {
	floor := 0
	for i := len(blocks) - 1; i >= 0; i-- {
		_, boundary := blocks[i].(*types.CompactionBoundaryBlock)
		_, marker := blocks[i].(*types.CompactionMarkerBlock)
		if boundary || marker {
			floor = i + 1
			break
		}
	}
	for i := len(blocks) - 1; i >= floor; i-- {
		if _, ok := blocks[i].(*types.ResponseBlock); !ok {
			continue
		}
		for j := i - 1; j >= floor; j-- {
			switch blocks[j].(type) {
			case *types.ThinkingBlock, *types.RedactedThinkingBlock, *types.TextBlock, *types.ToolCallBlock:
			case *types.UserBlock,
				*types.ContextBlock,
				*types.WakeupBlock,
				*types.SteerBlock,
				*types.AgentMessageBlock,
				*types.ResumeBlock,
				*types.ErrorBlock,
				*types.AbortBlock,
				*types.ResponseBlock,
				*types.CompactionBoundaryBlock,
				*types.CompactionMarkerBlock:
				return j + 1
			}
		}
		return floor
	}
	return -1
}
