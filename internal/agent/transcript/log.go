package transcript

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
)

// InterruptedTurnMessage is the error text for a session shut down during a turn.
const InterruptedTurnMessage = "The agent session was shut down while this turn was running."

// InterruptedCode is the code of an interrupted turn's error record.
const InterruptedCode = "interrupted"

// PendingCall is a provider-requested tool call with no result yet.
type PendingCall struct {
	ToolUseID string
	ToolName  string
	// Input is the JSON text the provider supplied.
	Input string
}

// Log owns ordered blocks and records every mutation in its patch journal.
// Its owner serializes access. Construct it with NewLog.
type Log struct{}

// NewLog creates a log with a fresh epoch and revision zero, including on restore.
// The caller supplies a nonnil identity source and clock.
func NewLog(blocks []core.Block, ids IDs, clock core.Clock) *Log { panic("not written: a-transcript") }

// Blocks returns the ordered transcript snapshot.
func (l *Log) Blocks() []core.Block { panic("not written: a-transcript") }

// Version returns the epoch and current published revision.
func (l *Log) Version() framewire.TranscriptVersion { panic("not written: a-transcript") }

// Find returns the last block with id, or nil.
func (l *Log) Find(id core.BlockID) core.Block { panic("not written: a-transcript") }

// TakePatches drains changes as one revision, or returns nil when nothing changed.
func (l *Log) TakePatches() *PatchBatch { panic("not written: a-transcript") }

// PushUser appends the input that opens a user turn.
func (l *Log) PushUser(turnID core.TurnID, model core.ModelSelection, content []core.UserContentBlock, preamble *string) core.BlockID {
	panic("not written: a-transcript")
}

// PushContext appends context from the named source.
func (l *Log) PushContext(turnID core.TurnID, model core.ModelSelection, source, text string) {
	panic("not written: a-transcript")
}

// PushSteer appends a human steer under its existing identity.
func (l *Log) PushSteer(id core.BlockID, turnID core.TurnID, model core.ModelSelection, content []core.UserContentBlock) {
	panic("not written: a-transcript")
}

// PushWakeup appends a fired yield wakeup under its existing identity.
func (l *Log) PushWakeup(id core.BlockID, turnID core.TurnID, model core.ModelSelection, placement core.WakeupPlacement) {
	panic("not written: a-transcript")
}

// PushAgentMessage appends an agent message under its message identity.
func (l *Log) PushAgentMessage(turnID core.TurnID, model core.ModelSelection, message core.AgentMessage) {
	panic("not written: a-transcript")
}

// PushResume records that a turn continues after a cut.
func (l *Log) PushResume(turnID core.TurnID, model core.ModelSelection) {
	panic("not written: a-transcript")
}

// MarkLatestAbortResumed marks the latest stopped marker as continued.
func (l *Log) MarkLatestAbortResumed() { panic("not written: a-transcript") }

// ReplaceAll publishes a history rewrite as one replace patch, superseding pending
// changes. The rewrite already saved its rows, so the batch marks none.
func (l *Log) ReplaceAll(blocks []core.Block) PatchBatch { panic("not written: a-transcript") }

// InsertCompactionBoundary inserts a summary where the retained history begins.
func (l *Log) InsertCompactionBoundary(index int, model core.ModelSelection, summary string, summaryTokens uint64) core.BlockID {
	panic("not written: a-transcript")
}

// PushCompactionMarker appends the estimate of what the boundary summarized.
func (l *Log) PushCompactionMarker(model core.ModelSelection, boundaryID core.BlockID, compactedTokens uint64) {
	panic("not written: a-transcript")
}

// PushAbort appends the stopped marker left by Stop.
func (l *Log) PushAbort(model core.ModelSelection) { panic("not written: a-transcript") }

// PushError appends a failed request or interrupted turn record.
func (l *Log) PushError(model core.ModelSelection, message string, code *string, diagnostics *core.ProviderErrorDiagnostics) {
	panic("not written: a-transcript")
}

// HasUserTurn reports whether a user block opened the turn.
func (l *Log) HasUserTurn(turn core.TurnID) bool { panic("not written: a-transcript") }

// EndsWithInterruption reports whether the last block records an interrupted turn.
func (l *Log) EndsWithInterruption() bool { panic("not written: a-transcript") }

// OpenThinking opens a reasoning block with text.
func (l *Log) OpenThinking(model core.ModelSelection, text string) {
	panic("not written: a-transcript")
}

// AppendThinking extends the last unsigned reasoning block or opens one.
func (l *Log) AppendThinking(model core.ModelSelection, text string) {
	panic("not written: a-transcript")
}

// SignThinking signs the latest reasoning block; without one it does nothing.
func (l *Log) SignThinking(signature string) { panic("not written: a-transcript") }

// PushRedactedThinking appends opaque reasoning data.
func (l *Log) PushRedactedThinking(model core.ModelSelection, data string) {
	panic("not written: a-transcript")
}

// AppendText extends the last incomplete text block or opens one.
func (l *Log) AppendText(model core.ModelSelection, text string) { panic("not written: a-transcript") }

// EndsWithOpenText reports whether the last block is incomplete answer text.
func (l *Log) EndsWithOpenText() bool { panic("not written: a-transcript") }

// CompleteTailText marks tail text complete and returns its identity, or nil
// when it is already complete, absent, or follows a still-executing call.
func (l *Log) CompleteTailText() *core.BlockID { panic("not written: a-transcript") }

// PushToolCall appends an executing call, retaining string input as text and
// representing null input as an empty object.
func (l *Log) PushToolCall(model core.ModelSelection, call provider.ToolCall) {
	panic("not written: a-transcript")
}

// PushResponse appends the usage of one answered request.
func (l *Log) PushResponse(model core.ModelSelection, usage core.TokenUsage) {
	panic("not written: a-transcript")
}

// PendingToolCalls returns every still-executing call in transcript order.
func (l *Log) PendingToolCalls() []PendingCall { panic("not written: a-transcript") }

// CompleteToolCall completes the latest executing call with this provider id.
// A provider may reuse an id across requests; an absent call is ignored.
func (l *Log) CompleteToolCall(toolUseID string, output []core.ToolResultContentBlock, isError bool, view core.ToolView) {
	panic("not written: a-transcript")
}
