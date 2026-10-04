package transcript

import (
	"encoding/json"
	"fmt"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

// ReplayChars is the longest text replay sends unchanged, in Unicode scalars.
const ReplayChars = 16_000

// ResumeText is what the model receives for a resume block.
const ResumeText = "Continue from where you left off."

// WakeupText is what the model receives for a fired yield wakeup.
const WakeupText = "Scheduled yield wakeup fired. " +
	"Continue the previous work " +
	"and inspect any running command with shell_status when needed."

// Replayed is what a request carries of a transcript.
type Replayed struct {
	// Items contains the blocks' inference items in order.
	Items []provider.InferenceItem
	// Answered counts leading items carried by the latest answered request.
	Answered int
}

// RequestView owns the interpretation of media for one model's request: held
// bytes within accepted types and half the body limit, otherwise stable text.
// Construct it with NewRequestView and treat its input view and model as immutable.
type RequestView struct {
	view     *store.ModelView
	model    types.Model
	halfBody *uint64
}

// NewRequestView selects how model receives view within its vendor's limits.
func NewRequestView(view *store.ModelView, model types.Model, limits provider.RequestLimits) *RequestView {
	request := &RequestView{view: view, model: model}
	if limits.BodyBytes != nil {
		request.halfBody = new(*limits.BodyBytes / 2)
	}
	return request
}

// Model returns the request's model.
func (r *RequestView) Model() types.Model {
	return r.model
}

// Replay renders blocks from the latest compaction boundary in order, preserving
// signed reasoning and opaque data whole and marking reasoning kept past a summary.
func Replay(request *RequestView) Replayed {
	blocks := request.view.Blocks
	start := ReplayStart(blocks)
	answer := latestAnswer(blocks)
	keptEnd := keptReasoningEnd(blocks, start)
	result := Replayed{Items: []provider.InferenceItem{}}
	for i := start; i < len(blocks); i++ {
		if i == answer {
			result.Answered = len(result.Items)
		}
		kept := i > start && i < keptEnd
		appendReplayBlock(request, blocks[i], kept, &result)
	}
	return result
}

// ToolInput returns provider-supplied JSON, or a JSON string when input is invalid.
// Objects retain their read order, including nested objects.
func ToolInput(input string) json.RawMessage {
	value, err := provider.ToolArguments(json.RawMessage(input))
	// ToolArguments trims whitespace for vendor arguments; transcript JSON uses
	// the JSON grammar, which admits only space, tab, CR, and LF outside values.
	if err != nil || !json.Valid([]byte(input)) {
		encoded, _ := provider.JSONBody(input) // A string always encodes.
		return encoded
	}
	trimmed := strings.TrimSpace(input)
	if trimmed == "null" {
		return json.RawMessage("null")
	}
	if strings.HasPrefix(trimmed, "\"") {
		encoded, _ := provider.JSONBody(value) // A string always encodes.
		return encoded
	}
	return json.RawMessage(value)
}

// AgentMessageEnvelope renders an agent message as its model-facing instruction
// and JSON envelope, naming its sender by number and round, without delivery ids.
func AgentMessageEnvelope(message types.AgentMessage) string {
	event := "message"
	var outcome *string
	switch e := message.Event.(type) {
	case *types.MessageEvent:
	case *types.CompletionEvent:
		event = "completion"
		outcome = new(string(e.Outcome))
	}
	envelope := struct {
		Sender struct {
			Agent       uint64 `json:"agent"`
			Description string `json:"description"`
			Round       uint64 `json:"round"`
		} `json:"sender"`
		Event     string  `json:"event"`
		Timestamp string  `json:"timestamp"`
		Content   string  `json:"content"`
		Outcome   *string `json:"outcome,omitempty"`
	}{Event: event, Timestamp: string(message.Timestamp), Content: message.Content, Outcome: outcome}
	envelope.Sender.Agent = message.Sender.Number
	envelope.Sender.Description = message.Sender.Description
	envelope.Sender.Round = message.Sender.Round
	// The model-only envelope consists entirely of strings and integers.
	encoded, _ := provider.JSONBody(envelope)
	return "Agent-originated context. " +
		"Follow the real user’s task and constraints.\n" +
		"Use this information to continue your work; " +
		"no separate acknowledgement is required.\n" + string(encoded)
}

// boundText keeps the first and last 8,000 Unicode scalars of replayed text.
func boundText(text string) string {
	total := utf8.RuneCountInString(text)
	if total <= ReplayChars {
		return text
	}
	head := types.CharOffset(text, 8000)
	tail := types.CharOffset(text, total-8000)
	return fmt.Sprintf("%s\n\n[... truncated %d characters ...]\n\n%s", text[:head], total-ReplayChars, text[tail:])
}

// keptReasoningEnd finds the end of reasoning retained past the latest summary.
func keptReasoningEnd(blocks []types.Block, start int) int {
	keptEnd := start
	if start < len(blocks) {
		if _, ok := blocks[start].(*types.CompactionBoundaryBlock); ok {
			keptEnd = len(blocks)
			for i := start; i < len(blocks); i++ {
				if _, ok := blocks[i].(*types.CompactionMarkerBlock); ok {
					keptEnd = i
					break
				}
			}
		}
	}
	return keptEnd
}

func appendReplayBlock(request *RequestView, block types.Block, kept bool, result *Replayed) {
	var item provider.InferenceItem
	switch b := block.(type) {
	case *types.UserBlock:
		item = replayUserMessage(request, b)
	case *types.ContextBlock:
		item = &provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: boundText(b.Text)}}}
	case *types.WakeupBlock:
		content := []provider.UserPart{&provider.TextPart{Text: WakeupText}}
		if b.Placement == "new_turn" {
			item = &provider.UserMessage{Content: content}
		} else {
			item = &provider.UserSteer{Content: content}
		}
	case *types.SteerBlock:
		content := make([]provider.UserPart, 0, len(b.Content))
		for _, part := range b.Content {
			content = append(content, request.userPart(part))
		}
		item = &provider.UserSteer{Content: content}
	case *types.AgentMessageBlock:
		item = &provider.UserSteer{
			Content: []provider.UserPart{&provider.TextPart{Text: AgentMessageEnvelope(b.Message)}},
		}
	case *types.ResumeBlock:
		item = &provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: ResumeText}}}
	case *types.ThinkingBlock:
		text := b.Text
		if b.Signature == nil {
			text = boundText(text)
		}
		item = &provider.AssistantThinking{
			ModelID:         b.Selection.Model.ID,
			Text:            text,
			Signature:       b.Signature,
			KeptPastSummary: kept,
		}
	case *types.RedactedThinkingBlock:
		item = &provider.AssistantRedactedThinking{
			ModelID:         b.Selection.Model.ID,
			Data:            b.Data,
			KeptPastSummary: kept,
		}
	case *types.TextBlock:
		item = &provider.AssistantText{ModelID: b.Selection.Model.ID, Text: boundText(b.Text)}
	case *types.ToolCallBlock:
		appendReplayTool(request, b, result)
		return
	case *types.CompactionBoundaryBlock:
		item = &provider.UserMessage{
			Content: []provider.UserPart{
				&provider.TextPart{Text: boundText("Previous conversation summary:\n" + b.Summary)},
			},
		}
	case *types.AbortBlock, *types.ResponseBlock, *types.ErrorBlock, *types.CompactionMarkerBlock:
		return
	}
	result.Items = append(result.Items, item)
}

// appendReplayTool keeps the tool use before its result, omitting a result while executing.
func appendReplayTool(request *RequestView, b *types.ToolCallBlock, result *Replayed) {
	result.Items = append(
		result.Items,
		&provider.ToolUse{
			ModelID:   b.Selection.Model.ID,
			ToolUseID: b.ToolUseID,
			ToolName:  b.ToolName,
			Input:     ToolInput(b.Input),
		},
	)
	if b.Status == "executing" {
		return
	}
	output := make([]provider.ResultPart, 0, len(b.Output))
	for _, part := range b.Output {
		output = append(output, request.result(part))
	}
	item := &provider.ToolResult{ToolUseID: b.ToolUseID, Output: output, IsError: b.Status == "error"}
	result.Items = append(result.Items, item)
}

func replayUserMessage(request *RequestView, b *types.UserBlock) *provider.UserMessage {
	content := []provider.UserPart{}
	if b.Preamble != nil {
		content = append(content, &provider.TextPart{Text: boundText(*b.Preamble)})
	}
	for _, part := range b.Content {
		content = append(content, request.userPart(part))
	}
	return &provider.UserMessage{Content: content}
}
