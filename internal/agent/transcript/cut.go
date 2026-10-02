package transcript

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import "github.com/wspl/demi/internal/core"

// ResumePoint identifies where re-inference restarts after an unfinished turn.
type ResumePoint struct {
	// Cut is the first index of the unfinished attempt's leftovers.
	Cut int
	// FullRerun says the whole attempt was leftovers, so resume reruns like retry.
	FullRerun bool
}

// Cut scans back over reasoning, errors, and blank text that nobody acted on.
// Reaching the input that opened the turn makes the cut a full rerun.
func Cut(blocks []core.Block) ResumePoint { panic("not written: a-transcript") }

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
func Rewind(blocks []core.Block) *Rewound { panic("not written: a-transcript") }

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
func (e CutError) Error() string { panic("not written: a-transcript") }

// BeforeUser returns the blocks before the editable user target.
func BeforeUser(blocks []core.Block, target core.BlockID) ([]core.Block, error) {
	panic("not written: a-transcript")
}

// ThroughAssistant returns the blocks through a completed answer target,
// refusing a prefix with unfinished tool calls.
func ThroughAssistant(blocks []core.Block, target core.BlockID) ([]core.Block, error) {
	panic("not written: a-transcript")
}

// CompactionWindow is the half-open range of blocks the next pass summarizes.
type CompactionWindow struct {
	Start int
	Cut   int
}

// Window ends at the latest answered request's answer, or at unanswered input
// when no request has been answered since the last compaction.
func Window(blocks []core.Block) CompactionWindow { panic("not written: a-transcript") }

// LastAssistantText returns the last answer text from since onward, or empty.
func LastAssistantText(blocks []core.Block, since int) string { panic("not written: a-transcript") }

// ReplayStart returns the last compaction boundary's index, or zero.
func ReplayStart(blocks []core.Block) int { panic("not written: a-transcript") }

// OpensInputTurn reports whether a block opens an input turn for recovery and
// before-user command-state boundaries.
func OpensInputTurn(block core.Block) bool { panic("not written: a-transcript") }
