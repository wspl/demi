package anthropicapi

import (
	"context"
	"encoding/json/jsontext"
	"io"
	"iter"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/provider"
)

type toolBlock struct {
	id, name  string
	initial   *jsontext.Value
	arguments string
}
type mapper struct {
	tools  map[uint32]toolBlock
	usage  core.TokenUsage
	vendor provider.Vendor
}

func (m *mapper) merge(counts *usage) {
	if counts == nil {
		return
	}
	keep := func(target *uint64, count *uint64) {
		if count != nil && *count > 0 {
			*target = *count
		}
	}
	keep(&m.usage.InputTokens, counts.InputTokens)
	keep(&m.usage.OutputTokens, counts.OutputTokens)
	keep(&m.usage.CacheReadTokens, counts.CacheRead)
	keep(&m.usage.CacheWriteTokens, counts.CacheWrite)
}
func (m *mapper) frame(data string) (provider.ProviderEvent, bool, error) {
	tag, err := decode[streamTag]([]byte(data))
	if err != nil {
		return nil, true, err
	}
	switch tag.Type {
	case "message_start":
		event, err := decode[messageStart]([]byte(data))
		if err != nil {
			return nil, true, err
		}
		m.merge(event.Message.Usage)
	case "message_delta":
		event, err := decode[messageDelta]([]byte(data))
		if err != nil {
			return nil, true, err
		}
		m.merge(event.Usage)
	case "message_stop":
		return provider.Response{Usage: m.usage}, true, nil
	case "content_block_start":
		event, err := decode[blockStart]([]byte(data))
		if err != nil {
			return nil, true, err
		}
		out, err := m.start(event)
		return out, false, err
	case "content_block_delta":
		event, err := decode[blockDelta]([]byte(data))
		if err != nil {
			return nil, true, err
		}
		out, err := m.delta(event)
		return out, false, err
	case "content_block_stop":
		event, err := decode[blockStop]([]byte(data))
		if err != nil {
			return nil, true, err
		}
		tool, ok := m.tools[event.Index]
		if ok {
			delete(m.tools, event.Index)
			input := jsontext.Value(`{}`)
			if tool.arguments != "" {
				input = provider.ToolInput(tool.arguments)
			} else if tool.initial != nil {
				input = *tool.initial
			}
			return provider.ToolCall{ToolUseID: tool.id, ToolName: tool.name, Input: input}, false, nil
		}
	case "error":
		event, err := decode[errorEvent]([]byte(data))
		if err != nil {
			return nil, true, err
		}
		message := "Anthropic API stream error"
		var code *string
		if event.Message != nil && event.Message.Value != nil {
			message = *event.Message.Value
		}
		if event.Error != nil {
			if event.Error.Type != nil {
				code = event.Error.Type.Value
			}
			if event.Error.Message != nil && event.Error.Message.Value != nil {
				message = *event.Error.Message.Value
			}
		}
		name := "error"
		if code != nil {
			name = *code
		}
		failure := provider.ProviderFailure{Message: message, Code: provider.ClassifyVendorFailure(name, message), Diagnostics: &core.ProviderErrorDiagnostics{Source: core.FailureSourceStream, ProviderCode: code, Upstream: &data}}
		return provider.FailureEvent{Failure: failure.WithRetryWait(m.vendor.Reader, m.vendor.Clock.Now())}, true, nil
	}
	return nil, false, nil
}
func (m *mapper) start(event blockStart) (provider.ProviderEvent, error) {
	tag, err := decode[streamTag](event.ContentBlock)
	if err != nil {
		return nil, wire.In("content_block", err)
	}
	switch tag.Type {
	case "thinking":
		return provider.ThinkingStart{}, nil
	case "redacted_thinking":
		block, err := decode[redactedBlock](event.ContentBlock)
		if err != nil {
			return nil, wire.In("content_block", err)
		}
		return provider.RedactedThinking{Data: SignatureTag + block.Data}, nil
	case "text":
		block, err := decode[textBlock](event.ContentBlock)
		if err != nil {
			return nil, wire.In("content_block", err)
		}
		if block.Text != "" {
			return provider.TextDelta{Text: block.Text}, nil
		}
	case "tool_use":
		block, err := decode[toolBlockStart](event.ContentBlock)
		if err != nil {
			return nil, wire.In("content_block", err)
		}
		m.tools[event.Index] = toolBlock{id: block.ID, name: block.Name, initial: block.Input}
	}
	return nil, nil
}
func (m *mapper) delta(event blockDelta) (provider.ProviderEvent, error) {
	tag, err := decode[streamTag](event.Delta)
	if err != nil {
		return nil, wire.In("delta", err)
	}
	switch tag.Type {
	case "text_delta":
		piece, err := decode[textBlock](event.Delta)
		if err != nil {
			return nil, wire.In("delta", err)
		}
		if piece.Text != "" {
			return provider.TextDelta{Text: piece.Text}, nil
		}
	case "thinking_delta":
		piece, err := decode[thinkingDelta](event.Delta)
		if err != nil {
			return nil, wire.In("delta", err)
		}
		if piece.Thinking != "" {
			return provider.ThinkingDelta{Text: piece.Thinking}, nil
		}
	case "signature_delta":
		piece, err := decode[signatureDelta](event.Delta)
		if err != nil {
			return nil, wire.In("delta", err)
		}
		if piece.Signature != "" {
			return provider.ThinkingSignature{Signature: SignatureTag + piece.Signature}, nil
		}
	case "input_json_delta":
		piece, err := decode[inputDelta](event.Delta)
		if err != nil {
			return nil, wire.In("delta", err)
		}
		if block, ok := m.tools[event.Index]; ok {
			block.arguments += piece.PartialJSON
			m.tools[event.Index] = block
		}
	}
	return nil, nil
}
func mapSSE(ctx context.Context, body io.Reader, vendor provider.Vendor) iter.Seq[provider.ProviderEvent] {
	return func(yield func(provider.ProviderEvent) bool) {
		mapper := mapper{tools: make(map[uint32]toolBlock), vendor: vendor}
		for data, err := range provider.SSEData(body) {
			if ctx.Err() != nil {
				return
			}
			if err != nil {
				yield(provider.FailureEvent{Failure: provider.EventStreamFailure(vendor.Label, err)})
				return
			}
			event, ended, err := mapper.frame(data)
			if err != nil {
				yield(provider.FailureEvent{Failure: provider.Undecodable(vendor.Label, err, data)})
				return
			}
			if event != nil && !yield(event) {
				return
			}
			if ended {
				return
			}
		}
		if ctx.Err() == nil {
			yield(provider.Response{Usage: mapper.usage})
		}
	}
}
