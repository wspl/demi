package anthropicapi

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"strings"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

const label = "Anthropic"

// Run streams inference events for the request.
func (r *runtime) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	return func(yield func(provider.Event) bool) {
		emit := func(event provider.Event) bool { return ctx.Err() == nil && yield(event) }
		body, err := provider.EncodeBody(
			ctx,
			label,
			func() ([]byte, error) { return encodeRequest(request, r.shared.policy) },
		)
		if ctx.Err() != nil {
			return
		}
		if err != nil {
			var failure *provider.Failure
			if errors.As(err, &failure) {
				emit(&provider.Error{Failure: *failure})
			}
			return
		}
		req, err := http.NewRequestWithContext(ctx, http.MethodPost, r.shared.endpoint, bytes.NewReader(body))
		if err != nil {
			emit(&provider.Error{Failure: provider.RequestBuildFailure(label, err)})
			return
		}
		req.Header.Set("x-api-key", r.shared.key.Expose())
		req.Header.Set("anthropic-version", "2023-06-01")
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Accept", "text/event-stream")
		response, err := r.http.Do(req)
		if err != nil {
			emit(&provider.Error{Failure: provider.TransportFailure(label, err)})
			return
		}
		// HTTPFailure and SSEData own and close the body, including cancellation.
		if response.StatusCode < 200 || response.StatusCode >= 300 {
			emit(
				&provider.Error{
					Failure: provider.HTTPFailure(ctx, response, label, provider.ReadHTTPFailure, r.shared.clock),
				},
			)
			return
		}
		r.stream(ctx, response, emit)
	}
}

type mapped struct {
	event provider.Event
	last  bool
}
type streamMapper struct {
	tools map[uint32]*toolBlock
	usage types.TokenUsage
	clock types.Clock
}
type toolBlock struct {
	id      string
	name    string
	initial *provider.Reported[json.RawMessage]
	input   strings.Builder
}
type usage struct {
	Input   *uint64 `json:"input_tokens"`
	Output  *uint64 `json:"output_tokens"`
	Read    *uint64 `json:"cache_read_input_tokens"`
	Written *uint64 `json:"cache_creation_input_tokens"`
}

// merge retains counts when later frames omit them or report zero.
func (m *streamMapper) merge(u *usage) {
	if u == nil {
		return
	}
	if u.Input != nil && *u.Input > 0 {
		m.usage.InputTokens = *u.Input
	}
	if u.Output != nil && *u.Output > 0 {
		m.usage.OutputTokens = *u.Output
	}
	if u.Read != nil && *u.Read > 0 {
		m.usage.CacheReadTokens = *u.Read
	}
	if u.Written != nil && *u.Written > 0 {
		m.usage.CacheWriteTokens = *u.Written
	}
}

// frame decodes only the registered Messages events and their known fields.
func (m *streamMapper) frame(data string) (mapped, error) {
	result, _, err := provider.DecodeTagged(data, map[string]func(string) (mapped, error){
		"message_start": func(text string) (mapped, error) {
			value, err := provider.DecodeUntagged[struct {
				Message struct {
					Usage *usage `json:"usage"`
				} `json:"message"`
			}](text)
			if err == nil {
				m.merge(value.Message.Usage)
			}
			return mapped{}, err
		},
		"message_delta": func(text string) (mapped, error) {
			value, err := provider.DecodeUntagged[struct {
				Usage *usage `json:"usage"`
			}](text)
			if err == nil {
				m.merge(value.Usage)
			}
			return mapped{}, err
		},
		"message_stop": func(text string) (mapped, error) {
			_, err := provider.DecodeUntagged[struct{}](text)
			return mapped{event: &provider.Response{Usage: m.usage}, last: true}, err
		},
		"content_block_start": m.blockStart,
		"content_block_delta": m.blockDelta,
		"content_block_stop": func(text string) (mapped, error) {
			value, err := provider.DecodeUntagged[struct {
				Index uint32 `json:"index"`
			}](text)
			if err != nil {
				return mapped{}, err
			}
			b := m.tools[value.Index]
			if b == nil {
				return mapped{}, nil
			}
			delete(m.tools, value.Index)
			return mapped{event: b.call()}, nil
		},
		"error": func(text string) (mapped, error) { return m.failure(text, data) },
	})
	return result, err
}

// blockStart opens a tool input or emits a block's initial content.
func (m *streamMapper) blockStart(text string) (mapped, error) {
	value, err := provider.DecodeUntagged[struct {
		Index uint32          `json:"index"`
		Block json.RawMessage `json:"content_block"`
	}](text)
	if err != nil {
		return mapped{}, err
	}
	result, _, err := provider.DecodeTagged(string(value.Block), map[string]func(string) (mapped, error){
		"tool_use": func(text string) (mapped, error) {
			tool, err := provider.DecodeUntagged[struct {
				ID    provider.NonEmpty                   `json:"id"`
				Name  provider.NonEmpty                   `json:"name"`
				Input *provider.Reported[json.RawMessage] `json:"input"`
			}](text)
			if err == nil {
				m.tools[value.Index] = &toolBlock{id: string(tool.ID), name: string(tool.Name), initial: tool.Input}
			}
			return mapped{}, err
		},
		"thinking": func(text string) (mapped, error) {
			_, err := provider.DecodeUntagged[struct{}](text)
			return mapped{event: &provider.ThinkingStart{}}, err
		},
		"redacted_thinking": func(text string) (mapped, error) {
			b, err := provider.DecodeUntagged[struct {
				Data string `json:"data"`
			}](text)
			return mapped{event: &provider.RedactedThinking{Data: signatureTag + b.Data}}, err
		},
		"text": func(text string) (mapped, error) {
			b, err := provider.DecodeUntagged[struct {
				Text string `json:"text"`
			}](text)
			if b.Text == "" {
				return mapped{}, err
			}
			return mapped{event: &provider.TextDelta{Text: b.Text}}, err
		},
	})
	return result, err
}

// blockDelta extends the block identified by the frame's index.
func (m *streamMapper) blockDelta(text string) (mapped, error) {
	value, err := provider.DecodeUntagged[struct {
		Index uint32          `json:"index"`
		Delta json.RawMessage `json:"delta"`
	}](text)
	if err != nil {
		return mapped{}, err
	}
	result, _, err := provider.DecodeTagged(string(value.Delta), map[string]func(string) (mapped, error){
		"text_delta": func(text string) (mapped, error) {
			b, err := provider.DecodeUntagged[struct {
				Text string `json:"text"`
			}](text)
			if b.Text == "" {
				return mapped{}, err
			}
			return mapped{event: &provider.TextDelta{Text: b.Text}}, err
		},
		"thinking_delta": func(text string) (mapped, error) {
			b, err := provider.DecodeUntagged[struct {
				Thinking string `json:"thinking"`
			}](text)
			if b.Thinking == "" {
				return mapped{}, err
			}
			return mapped{event: &provider.ThinkingDelta{Text: b.Thinking}}, err
		},
		"signature_delta": func(text string) (mapped, error) {
			b, err := provider.DecodeUntagged[struct {
				Signature string `json:"signature"`
			}](text)
			if b.Signature == "" {
				return mapped{}, err
			}
			return mapped{event: &provider.ThinkingSignature{Signature: signatureTag + b.Signature}}, err
		},
		"input_json_delta": func(text string) (mapped, error) {
			b, err := provider.DecodeUntagged[struct {
				Partial string `json:"partial_json"`
			}](text)
			if tool := m.tools[value.Index]; err == nil && tool != nil {
				tool.input.WriteString(b.Partial)
			}
			return mapped{}, err
		},
	})
	return result, err
}

// call preserves invalid streamed tool input as the vendor's original text.
func (b *toolBlock) call() *provider.ToolCall {
	input := json.RawMessage(`{}`)
	if b.input.Len() != 0 {
		var value provider.Reported[json.RawMessage]
		if err := value.UnmarshalJSON([]byte(b.input.String())); err == nil && value.Value != nil {
			input = *value.Value
		} else {
			input, _ = provider.JSONBody(b.input.String()) // A string always encodes.
		}
	} else if b.initial != nil && b.initial.Value != nil {
		input = *b.initial.Value
	}
	return &provider.ToolCall{ToolUseID: b.id, ToolName: b.name, Input: input}
}

// failure retains a vendor error frame verbatim and classifies its reported text.
func (m *streamMapper) failure(text, original string) (mapped, error) {
	value, err := provider.DecodeUntagged[struct {
		Message provider.ReportedString `json:"message" wire:"optional"`
		Error   *struct {
			Kind    provider.ReportedString `json:"type" wire:"optional"`
			Message provider.ReportedString `json:"message" wire:"optional"`
		} `json:"error"`
	}](text)
	if err != nil {
		return mapped{}, err
	}
	var code *string
	message := value.Message.Value
	if value.Error != nil {
		code = value.Error.Kind.Value
		if value.Error.Message.Value != nil {
			message = value.Error.Message.Value
		}
	}
	fallback := "Anthropic API stream error"
	if message == nil {
		message = &fallback
	}
	classify := "error"
	if code != nil {
		classify = *code
	}
	failure := provider.Failure{
		Message:     *message,
		Code:        provider.ClassifyError(&classify, *message),
		Diagnostics: &types.ProviderErrorDiagnostics{Source: "stream", ProviderCode: code, Upstream: &original},
	}
	return mapped{
		event: &provider.Error{Failure: failure.WithRetryWait(provider.ReadHTTPFailure, m.clock.Now())},
		last:  true,
	}, nil
}

func (r *runtime) stream(ctx context.Context, response *http.Response, emit func(provider.Event) bool) {
	mapper := streamMapper{tools: make(map[uint32]*toolBlock), clock: r.shared.clock}
	for data, err := range provider.SSEData(ctx, response.Body) {
		if ctx.Err() != nil {
			return
		}
		if err != nil {
			emit(&provider.Error{Failure: provider.EventStreamFailure(label, err)})
			return
		}
		next, err := mapper.frame(data)
		if err != nil {
			emit(&provider.Error{Failure: provider.Undecodable(label, err, data)})
			return
		}
		if next.event != nil && !emit(next.event) {
			return
		}
		if next.last {
			return
		}
	}
	emit(&provider.Response{Usage: mapper.usage})
}
