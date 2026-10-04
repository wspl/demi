package transcript

import (
	"slices"

	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

// InterruptedTurnMessage is the error text for a session shut down during a turn.
const InterruptedTurnMessage = "The agent session was shut down while this turn was running."

// InterruptedCode is the code of an interrupted turn's error record.
const InterruptedCode = "interrupted"

// PendingCall is a provider-requested tool call with no result yet.
type PendingCall struct {
	// ToolUseID identifies the provider's tool invocation.
	ToolUseID string
	// ToolName names the requested tool.
	ToolName string
	// Input is the JSON text the provider supplied.
	Input string
}

// Log owns ordered blocks and records every mutation in its patch journal.
// Its owner serializes access. Construct it with NewLog.
// Block data passed in or returned is immutable; mutations use these methods.
type Log struct {
	blocks   []types.Block
	journal  journal
	revision uint64
	epoch    string
	ids      IDs
	clock    types.Clock
}

// NewLog creates a log with a fresh epoch and revision zero, including on restore.
// The caller supplies a nonnil identity source and clock.
func NewLog(blocks []types.Block, ids IDs, clock types.Clock) *Log {
	return &Log{blocks: append([]types.Block{}, blocks...), epoch: ids.NextID(), ids: ids, clock: clock}
}

// Blocks returns the ordered transcript snapshot.
func (l *Log) Blocks() []types.Block {
	return slices.Clone(l.blocks)
}

// Version returns the epoch and current published revision.
func (l *Log) Version() conversationproto.TranscriptVersion {
	return conversationproto.TranscriptVersion{Epoch: l.epoch, Revision: l.revision}
}

// Find returns the last block with id, or nil.
func (l *Log) Find(id types.BlockID) types.Block {
	for i := len(l.blocks) - 1; i >= 0; i-- {
		if l.blocks[i].ID() == id {
			return l.blocks[i]
		}
	}
	return nil
}

// TakePatches drains changes as one revision, and returns false when nothing changed.
func (l *Log) TakePatches() (PatchBatch, bool) {
	if len(l.journal.patches) == 0 {
		return PatchBatch{}, false
	}
	l.revision++
	batch := PatchBatch{
		Revision: l.revision,
		Patches:  l.journal.patches,
		Touched:  l.journal.touched,
		Rows:     l.journal.rows,
	}
	l.journal = journal{}
	return batch, true
}

// PushUser appends the input that opens a user turn.
func (l *Log) PushUser(
	turnID types.TurnID,
	model types.ModelSelection,
	content []types.UserContentBlock,
	preamble *string,
) types.BlockID {
	id := l.nextBlockID()
	l.append(
		&types.UserBlock{
			BlockID:   id,
			TurnID:    turnID,
			Timestamp: l.clock.Now(),
			Selection: model,
			Content:   append([]types.UserContentBlock{}, content...),
			Preamble:  preamble,
		},
	)
	return id
}

// PushContext appends context from the named source.
func (l *Log) PushContext(turnID types.TurnID, model types.ModelSelection, source, text string) {
	l.append(
		&types.ContextBlock{
			BlockID:   l.nextBlockID(),
			TurnID:    turnID,
			Timestamp: l.clock.Now(),
			Selection: model,
			Source:    source,
			Text:      text,
		},
	)
}

// PushSteer appends a human steer under its existing identity.
func (l *Log) PushSteer(
	id types.BlockID,
	turnID types.TurnID,
	model types.ModelSelection,
	content []types.UserContentBlock,
) {
	l.append(
		&types.SteerBlock{
			BlockID:   id,
			TurnID:    turnID,
			Timestamp: l.clock.Now(),
			Selection: model,
			Content:   append([]types.UserContentBlock{}, content...),
		},
	)
}

// PushWakeup appends a fired yield wakeup under its existing identity.
func (l *Log) PushWakeup(
	id types.BlockID,
	turnID types.TurnID,
	model types.ModelSelection,
	placement types.WakeupPlacement,
) {
	l.append(
		&types.WakeupBlock{
			BlockID:   id,
			TurnID:    turnID,
			Timestamp: l.clock.Now(),
			Selection: model,
			Placement: placement,
		},
	)
}

// PushAgentMessage appends an agent message under its message identity.
func (l *Log) PushAgentMessage(turnID types.TurnID, model types.ModelSelection, message types.AgentMessage) {
	l.append(
		&types.AgentMessageBlock{
			BlockID:   message.ID,
			TurnID:    turnID,
			Timestamp: l.clock.Now(),
			Selection: model,
			Message:   message,
		},
	)
}

// PushResume records that a turn continues after a cut.
func (l *Log) PushResume(turnID types.TurnID, model types.ModelSelection) {
	l.append(&types.ResumeBlock{BlockID: l.nextBlockID(), TurnID: turnID, Timestamp: l.clock.Now(), Selection: model})
}

// MarkLatestAbortResumed marks the latest stopped marker as continued.
func (l *Log) MarkLatestAbortResumed() {
	for i := len(l.blocks) - 1; i >= 0; i-- {
		if block, ok := l.blocks[i].(*types.AbortBlock); ok {
			next := *block
			next.IsResumed = true
			l.replace(i, &next)
			return
		}
	}
}

// ReplaceAll publishes a history rewrite as one replace patch, superseding pending
// changes. The rewrite already saved its rows, so the batch marks none.
func (l *Log) ReplaceAll(blocks []types.Block) PatchBatch {
	l.journal = journal{}
	l.revision++
	l.blocks = append([]types.Block{}, blocks...)
	return PatchBatch{
		Revision: l.revision,
		Patches:  []conversationproto.TranscriptPatch{&conversationproto.ReplacePatch{Value: slices.Clone(l.blocks)}},
		Touched:  []types.BlockID{},
	}
}

// InsertCompactionBoundary inserts a summary where the retained history begins.
func (l *Log) InsertCompactionBoundary(
	index int,
	model types.ModelSelection,
	summary string,
	summaryTokens uint64,
) types.BlockID {
	id := l.nextBlockID()
	block := &types.CompactionBoundaryBlock{
		BlockID:       id,
		Timestamp:     l.clock.Now(),
		Selection:     model,
		Summary:       summary,
		SummaryTokens: summaryTokens,
	}
	l.journal.add(index, block)
	l.blocks = slices.Insert(l.blocks, index, types.Block(block))
	return id
}

// PushCompactionMarker appends the estimate of what the boundary summarized.
func (l *Log) PushCompactionMarker(model types.ModelSelection, boundaryID types.BlockID, compactedTokens uint64) {
	l.append(
		&types.CompactionMarkerBlock{
			BlockID:         l.nextBlockID(),
			Timestamp:       l.clock.Now(),
			Selection:       model,
			BoundaryID:      boundaryID,
			CompactedTokens: compactedTokens,
		},
	)
}

// PushAbort appends the stopped marker left by Stop.
func (l *Log) PushAbort(model types.ModelSelection) {
	l.append(&types.AbortBlock{BlockID: l.nextBlockID(), Timestamp: l.clock.Now(), Selection: model})
}

// PushError appends a failed request or interrupted turn record.
func (l *Log) PushError(
	model types.ModelSelection,
	message string,
	code *string,
	diagnostics *types.ProviderErrorDiagnostics,
) {
	l.append(
		&types.ErrorBlock{
			BlockID:     l.nextBlockID(),
			Timestamp:   l.clock.Now(),
			Selection:   model,
			Message:     message,
			Code:        code,
			Diagnostics: diagnostics,
		},
	)
}

// HasUserTurn reports whether a user block opened the turn.
func (l *Log) HasUserTurn(turn types.TurnID) bool {
	for _, block := range l.blocks {
		if user, ok := block.(*types.UserBlock); ok && user.TurnID == turn {
			return true
		}
	}
	return false
}

// EndsWithInterruption reports whether the last block records an interrupted turn.
func (l *Log) EndsWithInterruption() bool {
	if len(l.blocks) == 0 {
		return false
	}
	block, ok := l.blocks[len(l.blocks)-1].(*types.ErrorBlock)
	return ok && block.Code != nil && *block.Code == InterruptedCode
}

// OpenThinking opens a reasoning block with text.
func (l *Log) OpenThinking(model types.ModelSelection, text string) {
	l.append(&types.ThinkingBlock{BlockID: l.nextBlockID(), Timestamp: l.clock.Now(), Selection: model, Text: text})
}

// AppendThinking extends the last unsigned reasoning block or opens one.
func (l *Log) AppendThinking(model types.ModelSelection, text string) {
	index := len(l.blocks) - 1
	if index >= 0 {
		if block, ok := l.blocks[index].(*types.ThinkingBlock); ok && block.Signature == nil {
			next := *block
			next.Text += text
			l.blocks[index] = &next
			l.journal.appendText(index, next.BlockID, text)
			return
		}
	}
	l.OpenThinking(model, text)
}

// SignThinking signs the latest reasoning block; without one it does nothing.
func (l *Log) SignThinking(signature string) {
	for i := len(l.blocks) - 1; i >= 0; i-- {
		if block, ok := l.blocks[i].(*types.ThinkingBlock); ok {
			next := *block
			next.Signature = new(signature)
			l.replace(i, &next)
			return
		}
	}
}

// PushRedactedThinking appends opaque reasoning data.
func (l *Log) PushRedactedThinking(model types.ModelSelection, data string) {
	l.append(
		&types.RedactedThinkingBlock{BlockID: l.nextBlockID(), Timestamp: l.clock.Now(), Selection: model, Data: data},
	)
}

// AppendText extends the last incomplete text block or opens one.
func (l *Log) AppendText(model types.ModelSelection, text string) {
	index := len(l.blocks) - 1
	if index >= 0 {
		if block, ok := l.blocks[index].(*types.TextBlock); ok && !block.Forkable {
			next := *block
			next.Text += text
			l.blocks[index] = &next
			l.journal.appendText(index, next.BlockID, text)
			return
		}
	}
	l.append(&types.TextBlock{BlockID: l.nextBlockID(), Timestamp: l.clock.Now(), Selection: model, Text: text})
}

// EndsWithOpenText reports whether the last block is incomplete answer text.
func (l *Log) EndsWithOpenText() bool {
	if len(l.blocks) == 0 {
		return false
	}
	block, ok := l.blocks[len(l.blocks)-1].(*types.TextBlock)
	return ok && !block.Forkable
}

// CompleteTailText marks tail text complete and returns its identity, or false
// when it is already complete, absent, or follows a still-executing call.
func (l *Log) CompleteTailText() (types.BlockID, bool) {
	index := len(l.blocks) - 1
	if index < 0 {
		return "", false
	}
	block, ok := l.blocks[index].(*types.TextBlock)
	if !ok || block.Forkable {
		return "", false
	}
	for _, prior := range l.blocks[:index] {
		if call, ok := prior.(*types.ToolCallBlock); ok && call.Status == "executing" {
			return "", false
		}
	}
	next := *block
	next.Forkable = true
	l.replace(index, &next)
	return next.BlockID, true
}

// PushToolCall appends an executing call, retaining string input as text and
// representing null input as an empty object.
func (l *Log) PushToolCall(model types.ModelSelection, call provider.ToolCall) {
	input, err := provider.ToolArguments(call.Input)
	if err != nil {
		// Provider events normally contain validated JSON. Preserve malformed raw
		// input as text too, so the tool reports it rather than losing the call.
		input = string(call.Input)
	}
	l.append(
		&types.ToolCallBlock{
			BlockID:   l.nextBlockID(),
			Timestamp: l.clock.Now(),
			Selection: model,
			ToolUseID: call.ToolUseID,
			ToolName:  call.ToolName,
			Input:     input,
			Status:    "executing",
			Output:    []types.ToolResultContentBlock{},
		},
	)
}

// PushResponse appends the usage of one answered request.
func (l *Log) PushResponse(model types.ModelSelection, usage types.TokenUsage) {
	l.append(&types.ResponseBlock{BlockID: l.nextBlockID(), Timestamp: l.clock.Now(), Selection: model, Usage: usage})
}

// PendingToolCalls returns every still-executing call in transcript order.
func (l *Log) PendingToolCalls() []PendingCall {
	calls := []PendingCall{}
	for _, block := range l.blocks {
		if call, ok := block.(*types.ToolCallBlock); ok && call.Status == "executing" {
			calls = append(calls, PendingCall{ToolUseID: call.ToolUseID, ToolName: call.ToolName, Input: call.Input})
		}
	}
	return calls
}

// CompleteToolCall completes the latest executing call with this provider id.
// A provider may reuse an id across requests; an absent call is ignored.
func (l *Log) CompleteToolCall(
	toolUseID string,
	output []types.ToolResultContentBlock,
	isError bool,
	view types.ToolView,
) {
	for i := len(l.blocks) - 1; i >= 0; i-- {
		call, ok := l.blocks[i].(*types.ToolCallBlock)
		if !ok || call.Status != "executing" || call.ToolUseID != toolUseID {
			continue
		}
		next := *call
		next.Status = "completed"
		if isError {
			next.Status = "error"
		}
		next.Output = append([]types.ToolResultContentBlock{}, output...)
		next.View = view
		l.replace(i, &next)
		return
	}
}

// nextBlockID validates the nonempty identity guaranteed by the injected source.
func (l *Log) nextBlockID() types.BlockID {
	// IDs guarantees nonempty strings; Parse cannot fail for a conforming source.
	id, _ := types.ParseBlockID(l.ids.NextID())
	return id
}

// append records the transcript insertion before adding its immutable value.
func (l *Log) append(block types.Block) {
	l.journal.add(len(l.blocks), block)
	l.blocks = append(l.blocks, block)
}

// replace publishes a new block value; previous snapshots retain the old value.
func (l *Log) replace(index int, block types.Block) {
	l.blocks[index] = block
	l.journal.replace(index, block)
}
