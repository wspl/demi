package transcript

import (
	"slices"

	"github.com/wspl/demi/internal/core"
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
func Cut(blocks []core.Block) ResumePoint {
	for i := len(blocks) - 1; i >= 0; i-- {
		if OpensInputTurn(blocks[i]) {
			return ResumePoint{Cut: i + 1, FullRerun: true}
		}
		leftover := false
		switch b := blocks[i].(type) {
		case *core.ThinkingBlock, *core.RedactedThinkingBlock, *core.ErrorBlock:
			leftover = true
		case *core.TextBlock:
			leftover = core.IsBlank(b.Text)
		case *core.UserBlock, *core.ContextBlock, *core.WakeupBlock, *core.SteerBlock, *core.AgentMessageBlock, *core.ResumeBlock, *core.AbortBlock, *core.ToolCallBlock, *core.ResponseBlock, *core.CompactionBoundaryBlock, *core.CompactionMarkerBlock:
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
	Retained []core.Block
	// Input is the index of the opening input in Retained.
	Input int
	// Turn is the input's turn, which the rerun continues.
	Turn core.TurnID
}

// Rewind retains the last input turn for retry, or returns nil without one.
// The first agent message of a continuation also opens its turn.
func Rewind(blocks []core.Block) *Rewound {
	seen := map[core.TurnID]bool{}
	start := -1
	for i, b := range blocks {
		turn := turnOf(b)
		if turn == "" {
			continue
		}
		_, message := b.(*core.AgentMessageBlock)
		if OpensInputTurn(b) || (message && !seen[turn]) {
			start = i
		}
		seen[turn] = true
	}
	if start < 0 {
		return nil
	}
	turn := turnOf(blocks[start])
	retained := slices.Clone(blocks[:start+1])
	for _, b := range blocks[start+1:] {
		if _, ok := b.(*core.AgentMessageBlock); ok {
			retained = append(retained, b)
		}
		if steer, ok := b.(*core.SteerBlock); ok && steer.TurnID == turn {
			retained = append(retained, b)
		}
	}
	return &Rewound{Retained: retained, Input: start, Turn: turn}
}

// turnOf identifies the turn to which a transcript input belongs.
func turnOf(block core.Block) core.TurnID {
	switch b := block.(type) {
	case *core.UserBlock:
		return b.TurnID
	case *core.ContextBlock:
		return b.TurnID
	case *core.WakeupBlock:
		return b.TurnID
	case *core.SteerBlock:
		return b.TurnID
	case *core.AgentMessageBlock:
		return b.TurnID
	case *core.ResumeBlock:
		return b.TurnID
	case *core.ThinkingBlock, *core.RedactedThinkingBlock, *core.TextBlock, *core.ErrorBlock, *core.AbortBlock, *core.ToolCallBlock, *core.ResponseBlock, *core.CompactionBoundaryBlock, *core.CompactionMarkerBlock:
		return ""
	}
	return ""
}

// CutError identifies why a requested edit or Fork boundary is invalid.
type CutError string

const (
	// NotUserMessage rejects an edit target that is not a user message.
	NotUserMessage CutError = "The edit target must be a user message"
	// NotCompletedText rejects a Fork target that is not completed answer text.
	NotCompletedText CutError = "The Fork target must be a completed assistant message"
	// UnfinishedToolCalls rejects a Fork prefix with an executing tool call.
	UnfinishedToolCalls CutError = "The Fork boundary contains unfinished tool calls"
)

// Error returns the user-facing reason the cut was refused.
func (e CutError) Error() string { return string(e) }

// BeforeUser returns the blocks before the editable user target.
func BeforeUser(blocks []core.Block, target core.BlockID) ([]core.Block, error) {
	for i, b := range blocks {
		if b.ID() != target {
			continue
		}
		if !b.IsEditable() {
			break
		}
		return slices.Clone(blocks[:i]), nil
	}
	return nil, NotUserMessage
}

// ThroughAssistant returns the blocks through a completed answer target,
// refusing a prefix with unfinished tool calls.
func ThroughAssistant(blocks []core.Block, target core.BlockID) ([]core.Block, error) {
	for i, b := range blocks {
		if b.ID() != target {
			continue
		}
		text, ok := b.(*core.TextBlock)
		if !ok || !text.Forkable {
			break
		}
		for _, prior := range blocks[:i+1] {
			if call, ok := prior.(*core.ToolCallBlock); ok && call.Status == "executing" {
				return nil, UnfinishedToolCalls
			}
		}
		return slices.Clone(blocks[:i+1]), nil
	}
	return nil, NotCompletedText
}

// CompactionWindow is the half-open range of blocks the next pass summarizes.
type CompactionWindow struct {
	Start int
	Cut   int
}

// Window ends at the latest answered request's answer, or at unanswered input
// when no request has been answered since the last compaction.
func Window(blocks []core.Block) CompactionWindow {
	start := ReplayStart(blocks)
	cut := latestAnswer(blocks)
	if cut < 0 {
		answered := 0
		for i := len(blocks) - 1; i >= 0; i-- {
			if _, ok := blocks[i].(*core.ResponseBlock); ok {
				answered = i + 1
				break
			}
		}
		cut = answered
		for i := len(blocks) - 1; i >= answered; i-- {
			if _, ok := blocks[i].(*core.UserBlock); ok {
				cut = i
				break
			}
		}
		cut = max(cut, start)
	}
	return CompactionWindow{Start: start, Cut: cut}
}

// LastAssistantText returns the last answer text from since onward, or empty.
func LastAssistantText(blocks []core.Block, since int) string {
	for i := len(blocks) - 1; i >= max(since, 0); i-- {
		if b, ok := blocks[i].(*core.TextBlock); ok {
			return b.Text
		}
	}
	return ""
}

// ReplayStart returns the last compaction boundary's index, or zero.
func ReplayStart(blocks []core.Block) int {
	for i := len(blocks) - 1; i >= 0; i-- {
		if _, ok := blocks[i].(*core.CompactionBoundaryBlock); ok {
			return i
		}
	}
	return 0
}

// OpensInputTurn reports whether a block opens an input turn for recovery and
// before-user command-state boundaries.
func OpensInputTurn(block core.Block) bool {
	switch b := block.(type) {
	case *core.UserBlock, *core.ContextBlock:
		return true
	case *core.WakeupBlock:
		return b.Placement == "new_turn"
	case *core.SteerBlock, *core.AgentMessageBlock, *core.ResumeBlock, *core.ThinkingBlock, *core.RedactedThinkingBlock, *core.TextBlock, *core.ErrorBlock, *core.AbortBlock, *core.ToolCallBlock, *core.ResponseBlock, *core.CompactionBoundaryBlock, *core.CompactionMarkerBlock:
		return false
	}
	return false
}

// latestAnswer locates the answer to the latest request after compaction, or -1.
func latestAnswer(blocks []core.Block) int {
	floor := 0
	for i := len(blocks) - 1; i >= 0; i-- {
		_, boundary := blocks[i].(*core.CompactionBoundaryBlock)
		_, marker := blocks[i].(*core.CompactionMarkerBlock)
		if boundary || marker {
			floor = i + 1
			break
		}
	}
	for i := len(blocks) - 1; i >= floor; i-- {
		if _, ok := blocks[i].(*core.ResponseBlock); !ok {
			continue
		}
		for j := i - 1; j >= floor; j-- {
			switch blocks[j].(type) {
			case *core.ThinkingBlock, *core.RedactedThinkingBlock, *core.TextBlock, *core.ToolCallBlock:
			case *core.UserBlock, *core.ContextBlock, *core.WakeupBlock, *core.SteerBlock, *core.AgentMessageBlock, *core.ResumeBlock, *core.ErrorBlock, *core.AbortBlock, *core.ResponseBlock, *core.CompactionBoundaryBlock, *core.CompactionMarkerBlock:
				return j + 1
			}
		}
		return floor
	}
	return -1
}
