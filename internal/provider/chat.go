package provider

import (
	"context"
	"io"
	"sort"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/core"
)

// ChatChunk is a Chat Completions streamed chunk; absent or null usage is allowed.
type ChatChunk struct {
	Choices *[]ChatChoice `json:"choices"`
	Usage   *ChatUsage    `json:"usage"`
	Error   *ChatError    `json:"error"`
}

// ChatChoice carries an optional increment and completion reason.
type ChatChoice struct {
	Delta        *ChatDelta `json:"delta"`
	FinishReason *string    `json:"finish_reason"`
}

// ChatDelta carries answer, reasoning and tool-call increments.
type ChatDelta struct {
	Content          *string              `json:"content"`
	ReasoningContent *string              `json:"reasoning_content"`
	ToolCalls        *[]ChatToolCallDelta `json:"tool_calls"`
}

// ChatToolCallDelta groups increments by index; ID and name may arrive separately.
type ChatToolCallDelta struct {
	Index    *uint32            `json:"index"`
	ID       *string            `json:"id"`
	Function *ChatFunctionDelta `json:"function"`
}

// ChatFunctionDelta carries an optional function name and argument fragment.
type ChatFunctionDelta struct {
	Name      *string `json:"name"`
	Arguments *string `json:"arguments"`
}

// ChatUsage holds whole-number token counts, whose prompt includes the cache.
type ChatUsage struct {
	PromptTokens        *uint64                  `json:"prompt_tokens"`
	CompletionTokens    *uint64                  `json:"completion_tokens"`
	PromptTokensDetails *ChatPromptTokensDetails `json:"prompt_tokens_details"`
}

// ChatPromptTokensDetails holds the cached prefix count.
type ChatPromptTokensDetails struct {
	CachedTokens *uint64 `json:"cached_tokens"`
}

// TokenUsage separates the cached prefix from input tokens.
func (u ChatUsage) TokenUsage() core.TokenUsage {
	var read *uint64
	if u.PromptTokensDetails != nil {
		read = u.PromptTokensDetails.CachedTokens
	}
	return usageWithCachedInput(u.PromptTokens, u.CompletionTokens, read, nil)
}

// ChatError reports vendor failure fields without requiring them to be strings.
type ChatError struct {
	Message ReportedString `json:"message" wire:"optional"`
	Code    ReportedString `json:"code"    wire:"optional"`
	Kind    ReportedString `json:"type"    wire:"optional"`
}

// DecodeChatChunk decodes the fields of a vendor chunk, naming malformed fields.
func DecodeChatChunk(data string) (ChatChunk, error) { return DecodeUntagged[ChatChunk](data) }

// MapChatSSE maps one body to run events and closes it on every exit. A clean
// end or DONE flushes collected calls followed by one response.
func MapChatSSE(ctx context.Context, body io.ReadCloser, vendor Vendor) Run {
	return func(yield func(Event) bool) {
		mapper := chatMapper{calls: make(map[uint32]*collectedCall)}
		for data, err := range SSEData(ctx, body) {
			if err != nil {
				yield(&Error{Failure: EventStreamFailure(vendor.Label, err)})
				return
			}
			events, ended := mapper.frame(data, vendor)
			for _, event := range events {
				if ctx.Err() != nil || !yield(event) {
					return
				}
			}
			if ended {
				return
			}
		}
		if ctx.Err() != nil {
			return
		}
		for _, event := range mapper.finish() {
			if !yield(event) {
				return
			}
		}
	}
}

type collectedCall struct {
	id        string
	name      string
	arguments string
}
type chatMapper struct {
	calls           map[uint32]*collectedCall
	thinkingStarted bool
	usage           core.TokenUsage
}

func (m *chatMapper) frame(data string, vendor Vendor) ([]Event, bool) {
	if strings.TrimSpace(data) == "[DONE]" {
		return m.finish(), true
	}
	chunk, err := DecodeChatChunk(data)
	if err != nil {
		return []Event{&Error{Failure: Undecodable(vendor.Label, err, data)}}, true
	}
	if chunk.Error != nil {
		message := vendor.Label + " stream error"
		if chunk.Error.Message.Value != nil {
			message = *chunk.Error.Message.Value
		}
		code := chunk.Error.Code.Value
		if code == nil {
			code = chunk.Error.Kind.Value
		}
		failure := Failure{
			Message:     message,
			Code:        ClassifyError(code, message),
			Diagnostics: &core.ProviderErrorDiagnostics{Source: "stream", ProviderCode: code, Upstream: &data},
		}
		return []Event{&Error{Failure: failure.WithRetryWait(vendor.Reader, vendor.Clock.Now())}}, true
	}
	if chunk.Usage != nil {
		m.usage = chunk.Usage.TokenUsage()
	}
	var out []Event
	if chunk.Choices == nil {
		return out, false
	}
	for _, choice := range *chunk.Choices {
		out = append(out, m.delta(choice.Delta)...)
		if choice.FinishReason != nil && *choice.FinishReason == "tool_calls" {
			out = append(out, m.flush()...)
		}
	}
	return out, false
}

func (m *chatMapper) collect(delta ChatToolCallDelta) {
	index := uint32(min(uint64(len(m.calls)), uint64(^uint32(0))))
	if delta.Index != nil {
		index = *delta.Index
	}
	call := m.calls[index]
	if call == nil {
		call = &collectedCall{}
		m.calls[index] = call
	}
	if delta.ID != nil && *delta.ID != "" {
		call.id = *delta.ID
	}
	if function := delta.Function; function != nil {
		if function.Name != nil && *function.Name != "" {
			call.name = *function.Name
		}
		if function.Arguments != nil {
			call.arguments += *function.Arguments
		}
	}
}

func (m *chatMapper) flush() []Event {
	indices := make([]uint32, 0, len(m.calls))
	for index := range m.calls {
		indices = append(indices, index)
	}
	sort.Slice(indices, func(i, j int) bool { return indices[i] < indices[j] })
	var out []Event
	for _, index := range indices {
		call := m.calls[index]
		if call.name == "" {
			continue
		}
		id := call.id
		if id == "" {
			id = "tool_call_" + strconv.FormatUint(uint64(index), 10)
		}
		arguments := call.arguments
		if arguments == "" {
			arguments = "{}"
		}
		out = append(out, &ToolCall{ToolUseID: id, ToolName: call.name, Input: toolInput(arguments)})
	}
	clear(m.calls)
	return out
}
func (m *chatMapper) finish() []Event { return append(m.flush(), &Response{Usage: m.usage}) }

func (m *chatMapper) delta(delta *ChatDelta) []Event {
	if delta == nil {
		return nil
	}
	var out []Event
	if text := delta.ReasoningContent; text != nil && *text != "" {
		if !m.thinkingStarted {
			m.thinkingStarted = true
			out = append(out, &ThinkingStart{})
		}
		out = append(out, &ThinkingDelta{Text: *text})
	}
	if text := delta.Content; text != nil && *text != "" {
		out = append(out, &TextDelta{Text: *text})
	}
	if delta.ToolCalls != nil {
		for _, call := range *delta.ToolCalls {
			m.collect(call)
		}
	}
	return out
}
