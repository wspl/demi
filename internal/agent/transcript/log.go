package transcript

import (
	"slices"

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
	blocks   []core.Block
	journal  journal
	revision uint64
	epoch    string
	ids      IDs
	clock    core.Clock
}

// NewLog creates a log with a fresh epoch and revision zero, including on restore.
// The caller supplies a nonnil identity source and clock.
func NewLog(blocks []core.Block, ids IDs, clock core.Clock) *Log {
	return &Log{blocks: append([]core.Block{}, blocks...), epoch: ids.NextID(), ids: ids, clock: clock}
}

// Blocks returns the ordered transcript snapshot.
func (l *Log) Blocks() []core.Block { return slices.Clone(l.blocks) }

// Version returns the epoch and current published revision.
func (l *Log) Version() framewire.TranscriptVersion {
	return framewire.TranscriptVersion{Epoch: l.epoch, Revision: l.revision}
}

// Find returns the last block with id, or nil.
func (l *Log) Find(id core.BlockID) core.Block {
	for i := len(l.blocks) - 1; i >= 0; i-- {
		if l.blocks[i].ID() == id {
			return l.blocks[i]
		}
	}
	return nil
}

// TakePatches drains changes as one revision, or returns nil when nothing changed.
func (l *Log) TakePatches() *PatchBatch {
	if len(l.journal.patches) == 0 {
		return nil
	}
	l.revision++
	batch := &PatchBatch{
		Revision: l.revision,
		Patches:  l.journal.patches,
		Touched:  l.journal.touched,
		Rows:     l.journal.rows,
	}
	l.journal = journal{}
	return batch
}

// PushUser appends the input that opens a user turn.
func (l *Log) PushUser(
	turnID core.TurnID,
	model core.ModelSelection,
	content []core.UserContentBlock,
	preamble *string,
) core.BlockID {
	id := l.nextBlockID()
	l.append(
		&core.UserBlock{
			BlockID:   id,
			TurnID:    turnID,
			Timestamp: l.clock.Now(),
			Selection: model,
			Content:   append([]core.UserContentBlock{}, content...),
			Preamble:  preamble,
		},
	)
	return id
}

// PushContext appends context from the named source.
func (l *Log) PushContext(turnID core.TurnID, model core.ModelSelection, source, text string) {
	l.append(
		&core.ContextBlock{
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
	id core.BlockID,
	turnID core.TurnID,
	model core.ModelSelection,
	content []core.UserContentBlock,
) {
	l.append(
		&core.SteerBlock{
			BlockID:   id,
			TurnID:    turnID,
			Timestamp: l.clock.Now(),
			Selection: model,
			Content:   append([]core.UserContentBlock{}, content...),
		},
	)
}

// PushWakeup appends a fired yield wakeup under its existing identity.
func (l *Log) PushWakeup(
	id core.BlockID,
	turnID core.TurnID,
	model core.ModelSelection,
	placement core.WakeupPlacement,
) {
	l.append(
		&core.WakeupBlock{
			BlockID:   id,
			TurnID:    turnID,
			Timestamp: l.clock.Now(),
			Selection: model,
			Placement: placement,
		},
	)
}

// PushAgentMessage appends an agent message under its message identity.
func (l *Log) PushAgentMessage(turnID core.TurnID, model core.ModelSelection, message core.AgentMessage) {
	l.append(
		&core.AgentMessageBlock{
			BlockID:   message.ID,
			TurnID:    turnID,
			Timestamp: l.clock.Now(),
			Selection: model,
			Message:   message,
		},
	)
}

// PushResume records that a turn continues after a cut.
func (l *Log) PushResume(turnID core.TurnID, model core.ModelSelection) {
	l.append(&core.ResumeBlock{BlockID: l.nextBlockID(), TurnID: turnID, Timestamp: l.clock.Now(), Selection: model})
}

// MarkLatestAbortResumed marks the latest stopped marker as continued.
func (l *Log) MarkLatestAbortResumed() {
	for i := len(l.blocks) - 1; i >= 0; i-- {
		if block, ok := l.blocks[i].(*core.AbortBlock); ok {
			next := *block
			next.IsResumed = true
			l.replace(i, &next)
			return
		}
	}
}

// ReplaceAll publishes a history rewrite as one replace patch, superseding pending
// changes. The rewrite already saved its rows, so the batch marks none.
func (l *Log) ReplaceAll(blocks []core.Block) PatchBatch {
	l.journal = journal{}
	l.revision++
	l.blocks = append([]core.Block{}, blocks...)
	return PatchBatch{
		Revision: l.revision,
		Patches:  []framewire.TranscriptPatch{&framewire.ReplacePatch{Value: slices.Clone(l.blocks)}},
		Touched:  []core.BlockID{},
	}
}

// InsertCompactionBoundary inserts a summary where the retained history begins.
func (l *Log) InsertCompactionBoundary(
	index int,
	model core.ModelSelection,
	summary string,
	summaryTokens uint64,
) core.BlockID {
	id := l.nextBlockID()
	block := &core.CompactionBoundaryBlock{
		BlockID:       id,
		Timestamp:     l.clock.Now(),
		Selection:     model,
		Summary:       summary,
		SummaryTokens: summaryTokens,
	}
	l.journal.add(index, block)
	l.blocks = slices.Insert(l.blocks, index, core.Block(block))
	return id
}

// PushCompactionMarker appends the estimate of what the boundary summarized.
func (l *Log) PushCompactionMarker(model core.ModelSelection, boundaryID core.BlockID, compactedTokens uint64) {
	l.append(
		&core.CompactionMarkerBlock{
			BlockID:         l.nextBlockID(),
			Timestamp:       l.clock.Now(),
			Selection:       model,
			BoundaryID:      boundaryID,
			CompactedTokens: compactedTokens,
		},
	)
}

// PushAbort appends the stopped marker left by Stop.
func (l *Log) PushAbort(model core.ModelSelection) {
	l.append(&core.AbortBlock{BlockID: l.nextBlockID(), Timestamp: l.clock.Now(), Selection: model})
}

// PushError appends a failed request or interrupted turn record.
func (l *Log) PushError(
	model core.ModelSelection,
	message string,
	code *string,
	diagnostics *core.ProviderErrorDiagnostics,
) {
	l.append(
		&core.ErrorBlock{
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
func (l *Log) HasUserTurn(turn core.TurnID) bool {
	for _, block := range l.blocks {
		if user, ok := block.(*core.UserBlock); ok && user.TurnID == turn {
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
	block, ok := l.blocks[len(l.blocks)-1].(*core.ErrorBlock)
	return ok && block.Code != nil && *block.Code == InterruptedCode
}

// OpenThinking opens a reasoning block with text.
func (l *Log) OpenThinking(model core.ModelSelection, text string) {
	l.append(&core.ThinkingBlock{BlockID: l.nextBlockID(), Timestamp: l.clock.Now(), Selection: model, Text: text})
}

// AppendThinking extends the last unsigned reasoning block or opens one.
func (l *Log) AppendThinking(model core.ModelSelection, text string) {
	index := len(l.blocks) - 1
	if index >= 0 {
		if block, ok := l.blocks[index].(*core.ThinkingBlock); ok && block.Signature == nil {
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
		if block, ok := l.blocks[i].(*core.ThinkingBlock); ok {
			next := *block
			next.Signature = new(signature)
			l.replace(i, &next)
			return
		}
	}
}

// PushRedactedThinking appends opaque reasoning data.
func (l *Log) PushRedactedThinking(model core.ModelSelection, data string) {
	l.append(
		&core.RedactedThinkingBlock{BlockID: l.nextBlockID(), Timestamp: l.clock.Now(), Selection: model, Data: data},
	)
}

// AppendText extends the last incomplete text block or opens one.
func (l *Log) AppendText(model core.ModelSelection, text string) {
	index := len(l.blocks) - 1
	if index >= 0 {
		if block, ok := l.blocks[index].(*core.TextBlock); ok && !block.Forkable {
			next := *block
			next.Text += text
			l.blocks[index] = &next
			l.journal.appendText(index, next.BlockID, text)
			return
		}
	}
	l.append(&core.TextBlock{BlockID: l.nextBlockID(), Timestamp: l.clock.Now(), Selection: model, Text: text})
}

// EndsWithOpenText reports whether the last block is incomplete answer text.
func (l *Log) EndsWithOpenText() bool {
	if len(l.blocks) == 0 {
		return false
	}
	block, ok := l.blocks[len(l.blocks)-1].(*core.TextBlock)
	return ok && !block.Forkable
}

// CompleteTailText marks tail text complete and returns its identity, or nil
// when it is already complete, absent, or follows a still-executing call.
func (l *Log) CompleteTailText() *core.BlockID {
	index := len(l.blocks) - 1
	if index < 0 {
		return nil
	}
	block, ok := l.blocks[index].(*core.TextBlock)
	if !ok || block.Forkable {
		return nil
	}
	for _, prior := range l.blocks[:index] {
		if call, ok := prior.(*core.ToolCallBlock); ok && call.Status == "executing" {
			return nil
		}
	}
	next := *block
	next.Forkable = true
	l.replace(index, &next)
	return new(next.BlockID)
}

// PushToolCall appends an executing call, retaining string input as text and
// representing null input as an empty object.
func (l *Log) PushToolCall(model core.ModelSelection, call provider.ToolCall) {
	input, err := provider.ToolArguments(call.Input)
	if err != nil {
		// Provider events normally contain validated JSON. Preserve malformed raw
		// input as text too, so the tool reports it rather than losing the call.
		input = string(call.Input)
	}
	l.append(
		&core.ToolCallBlock{
			BlockID:   l.nextBlockID(),
			Timestamp: l.clock.Now(),
			Selection: model,
			ToolUseID: call.ToolUseID,
			ToolName:  call.ToolName,
			Input:     input,
			Status:    "executing",
			Output:    []core.ToolResultContentBlock{},
		},
	)
}

// PushResponse appends the usage of one answered request.
func (l *Log) PushResponse(model core.ModelSelection, usage core.TokenUsage) {
	l.append(&core.ResponseBlock{BlockID: l.nextBlockID(), Timestamp: l.clock.Now(), Selection: model, Usage: usage})
}

// PendingToolCalls returns every still-executing call in transcript order.
func (l *Log) PendingToolCalls() []PendingCall {
	calls := []PendingCall{}
	for _, block := range l.blocks {
		if call, ok := block.(*core.ToolCallBlock); ok && call.Status == "executing" {
			calls = append(calls, PendingCall{ToolUseID: call.ToolUseID, ToolName: call.ToolName, Input: call.Input})
		}
	}
	return calls
}

// CompleteToolCall completes the latest executing call with this provider id.
// A provider may reuse an id across requests; an absent call is ignored.
func (l *Log) CompleteToolCall(
	toolUseID string,
	output []core.ToolResultContentBlock,
	isError bool,
	view core.ToolView,
) {
	for i := len(l.blocks) - 1; i >= 0; i-- {
		call, ok := l.blocks[i].(*core.ToolCallBlock)
		if !ok || call.Status != "executing" || call.ToolUseID != toolUseID {
			continue
		}
		next := *call
		next.Status = "completed"
		if isError {
			next.Status = "error"
		}
		next.Output = append([]core.ToolResultContentBlock{}, output...)
		next.View = view
		l.replace(i, &next)
		return
	}
}

// nextBlockID validates the nonempty identity guaranteed by the injected source.
func (l *Log) nextBlockID() core.BlockID {
	// IDs guarantees nonempty strings; Parse cannot fail for a conforming source.
	id, _ := core.ParseBlockID(l.ids.NextID())
	return id
}

// append records the transcript insertion before adding its immutable value.
func (l *Log) append(block core.Block) {
	l.journal.add(len(l.blocks), block)
	l.blocks = append(l.blocks, block)
}

// replace publishes a new block value; previous snapshots retain the old value.
func (l *Log) replace(index int, block core.Block) {
	l.blocks[index] = block
	l.journal.replace(index, block)
}
