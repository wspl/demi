package google

import (
	"context"

	"encoding/json/jsontext"

	"io"
	"iter"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

type mapper struct {
	thinkingOpen bool
	usage        core.TokenUsage
	vendor       provider.Vendor
}

func (m *mapper) frame(data string) ([]provider.ProviderEvent, bool) {
	chunk, err := decode[chunk]([]byte(data))
	if err != nil {
		return []provider.ProviderEvent{provider.FailureEvent{Failure: provider.Undecodable(m.vendor.Label, err, data)}}, true
	}
	if chunk.Error != nil {
		message := "Google API stream error"
		var status *string
		if chunk.Error.Message != nil && chunk.Error.Message.Value != nil {
			message = *chunk.Error.Message.Value
		}
		if chunk.Error.Status != nil {
			status = chunk.Error.Status.Value
		}
		code := ""
		if status != nil {
			code = *status
		}
		failure := provider.ProviderFailure{Message: message, Code: provider.ClassifyVendorFailure(code, message), Diagnostics: &core.ProviderErrorDiagnostics{Source: core.FailureSourceStream, ProviderCode: status, Upstream: &data}}
		return []provider.ProviderEvent{provider.FailureEvent{Failure: failure.WithRetryWait(m.vendor.Reader, m.vendor.Clock.Now())}}, true
	}
	if usage := chunk.Usage; usage != nil {
		m.usage = provider.UsageWithCachedInput(usage.Prompt, usage.Candidates, usage.Cached, nil)
		if usage.Thoughts != nil {
			m.usage.OutputTokens += *usage.Thoughts
		}
	}
	var events []provider.ProviderEvent
	if chunk.Candidates != nil {
		for _, candidate := range *chunk.Candidates {
			if candidate.Content == nil || candidate.Content.Parts == nil {
				continue
			}
			for _, part := range *candidate.Content.Parts {
				events = append(events, m.part(part)...)
			}
		}
	}
	return events, false
}
func (m *mapper) part(part responsePart) []provider.ProviderEvent {
	var events []provider.ProviderEvent
	if call := part.FunctionCall; call != nil {
		if part.ThoughtSignature != nil {
			if !m.thinkingOpen {
				events = append(events, provider.ThinkingStart{})
			}
			events = append(events, provider.ThinkingSignature{Signature: SignatureTag + *part.ThoughtSignature})
		}
		id := ""
		if call.ID != nil {
			id = *call.ID
		} else {
			id = provider.NewToolUseID(call.Name + "_")
		}
		input := jsontext.Value(`{}`)
		if call.Args != nil {
			input = *call.Args
		}
		events = append(events, provider.ToolCall{ToolUseID: id, ToolName: call.Name, Input: input})
		m.thinkingOpen = false
		return events
	}
	if part.Thought != nil && *part.Thought {
		if !m.thinkingOpen {
			events = append(events, provider.ThinkingStart{})
			m.thinkingOpen = true
		}
		if part.Text != nil && *part.Text != "" {
			events = append(events, provider.ThinkingDelta{Text: *part.Text})
		}
		return events
	}
	if part.ThoughtSignature != nil && m.thinkingOpen {
		events = append(events, provider.ThinkingSignature{Signature: SignatureTag + *part.ThoughtSignature})
	}
	if part.Text != nil && *part.Text != "" {
		m.thinkingOpen = false
		events = append(events, provider.TextDelta{Text: *part.Text})
	}
	return events
}

func mapSSE(ctx context.Context, body io.Reader, vendor provider.Vendor) iter.Seq[provider.ProviderEvent] {
	return func(yield func(provider.ProviderEvent) bool) {
		mapper := mapper{vendor: vendor}
		for data, err := range provider.SSEData(body) {
			if ctx.Err() != nil {
				return
			}
			if err != nil {
				yield(provider.FailureEvent{Failure: provider.EventStreamFailure(vendor.Label, err)})
				return
			}
			events, ended := mapper.frame(data)
			for _, event := range events {
				if ctx.Err() != nil || !yield(event) {
					return
				}
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
