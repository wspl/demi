package provider

import (
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"io"
	"iter"
	"slices"
	"strings"

	"github.com/wspl/demi/go/core"
)

type Vendor struct {
	Label  string
	Reader FailureReader
	Clock  core.Clock
}

func UsageWithCachedInput(input, output, read, written *uint64) core.TokenUsage {
	value := func(p *uint64) uint64 {
		if p == nil {
			return 0
		}
		return *p
	}
	in, r, w := value(input), value(read), value(written)
	in -= min(in, r)
	in -= min(in, w)
	return core.TokenUsage{InputTokens: in, OutputTokens: value(output), CacheReadTokens: r, CacheWriteTokens: w}
}
func ToolInput(text string) jsontext.Value {
	value := jsontext.Value(text)
	if value.IsValid() {
		return value
	}
	encoded, err := json.Marshal(text)
	if err != nil {
		panic(err)
	} // Strings always encode.
	return encoded
}
func (u ChatUsage) TokenUsage() core.TokenUsage {
	var cached *uint64
	if u.PromptTokensDetails != nil {
		cached = u.PromptTokensDetails.CachedTokens
	}
	return UsageWithCachedInput(u.PromptTokens, u.CompletionTokens, cached, nil)
}
func DecodeChatChunk(data []byte) (ChatChunk, error) { return decode[ChatChunk](data) }
func Undecodable(label string, err error, text string) ProviderFailure {
	return ProtocolFailure(fmt.Sprintf("%s API stream sent a frame Demi cannot read: %v", label, err), text)
}
func reported(value *ReportedString) *string {
	if value == nil {
		return nil
	}
	return value.Value
}
func chatFailure(err ChatError, vendor Vendor, data string) ProviderFailure {
	message := vendor.Label + " stream error"
	if text := reported(err.Message); text != nil {
		message = *text
	}
	code := reported(err.Code)
	if code == nil {
		code = reported(err.Type)
	}
	name := ""
	if code != nil {
		name = *code
	}
	failure := ProviderFailure{Message: message, Code: ClassifyVendorFailure(name, message), Diagnostics: &core.ProviderErrorDiagnostics{Source: core.FailureSourceStream, ProviderCode: code, Upstream: &data}}
	return failure.WithRetryWait(vendor.Reader, vendor.Clock.Now())
}

type collectedChatCall struct{ id, name, arguments string }
type ChatMapper struct {
	calls           map[uint32]collectedChatCall
	thinkingStarted bool
	usage           core.TokenUsage
}

func (m *ChatMapper) Frame(data string, vendor Vendor) ([]ProviderEvent, bool) {
	if strings.TrimSpace(data) == "[DONE]" {
		return m.Finish(), true
	}
	chunk, err := DecodeChatChunk([]byte(data))
	if err != nil {
		return []ProviderEvent{FailureEvent{Failure: Undecodable(vendor.Label, err, data)}}, true
	}
	if chunk.Error != nil {
		return []ProviderEvent{FailureEvent{Failure: chatFailure(*chunk.Error, vendor, data)}}, true
	}
	if chunk.Usage != nil {
		m.usage = chunk.Usage.TokenUsage()
	}
	var out []ProviderEvent
	if chunk.Choices == nil {
		return out, false
	}
	for _, choice := range *chunk.Choices {
		if delta := choice.Delta; delta != nil {
			if delta.ReasoningContent != nil && *delta.ReasoningContent != "" {
				if !m.thinkingStarted {
					m.thinkingStarted = true
					out = append(out, ThinkingStart{})
				}
				out = append(out, ThinkingDelta{Text: *delta.ReasoningContent})
			}
			if delta.Content != nil && *delta.Content != "" {
				out = append(out, TextDelta{Text: *delta.Content})
			}
			if delta.ToolCalls != nil {
				for _, call := range *delta.ToolCalls {
					m.collect(call)
				}
			}
		}
		if choice.FinishReason != nil && *choice.FinishReason == "tool_calls" {
			out = append(out, m.flush()...)
		}
	}
	return out, false
}
func (m *ChatMapper) collect(delta ChatToolDelta) {
	if m.calls == nil {
		m.calls = make(map[uint32]collectedChatCall)
	}
	index := uint32(min(uint64(len(m.calls)), uint64(1<<32-1)))
	if delta.Index != nil {
		index = *delta.Index
	}
	call := m.calls[index]
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
	m.calls[index] = call
}
func (m *ChatMapper) flush() []ProviderEvent {
	keys := make([]uint32, 0, len(m.calls))
	for key := range m.calls {
		keys = append(keys, key)
	}
	slices.Sort(keys)
	var out []ProviderEvent
	for _, key := range keys {
		call := m.calls[key]
		if call.name == "" {
			continue
		}
		if call.id == "" {
			call.id = fmt.Sprintf("tool_call_%d", key)
		}
		if call.arguments == "" {
			call.arguments = "{}"
		}
		out = append(out, ToolCall{ToolUseID: call.id, ToolName: call.name, Input: ToolInput(call.arguments)})
	}
	clear(m.calls)
	return out
}
func (m *ChatMapper) Finish() []ProviderEvent { return append(m.flush(), Response{Usage: m.usage}) }

// MapChatSSE consumes the response body. The request owning body must use ctx,
// so cancellation interrupts a blocked read as well as suppressing later events.
func MapChatSSE(ctx context.Context, body io.Reader, vendor Vendor) iter.Seq[ProviderEvent] {
	return func(yield func(ProviderEvent) bool) {
		var mapper ChatMapper
		for data, err := range SSEData(body) {
			if ctx.Err() != nil {
				return
			}
			if err != nil {
				yield(FailureEvent{Failure: EventStreamFailure(vendor.Label, err)})
				return
			}
			events, ended := mapper.Frame(data, vendor)
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
		for _, event := range mapper.Finish() {
			if !yield(event) {
				return
			}
		}
	}
}
