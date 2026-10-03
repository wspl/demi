package google

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// Run streams inference events for the request.
func (r *runtime) Run(ctx context.Context, request provider.InferenceRequest) provider.Run {
	return func(yield func(provider.Event) bool) {
		emit := func(event provider.Event) bool { return ctx.Err() == nil && yield(event) }
		if ctx.Err() != nil {
			return
		}
		body, err := provider.EncodeBody(ctx, "Google", func() ([]byte, error) { return encode(request) })
		if err != nil {
			var failure *provider.Failure
			if errors.As(err, &failure) {
				emit(&provider.Error{Failure: *failure})
			}
			return
		}
		if ctx.Err() != nil {
			return
		}
		req, err := http.NewRequestWithContext(
			ctx,
			http.MethodPost,
			r.shared.streamURL(request.ModelID),
			bytes.NewReader(body),
		)
		if err != nil {
			emit(&provider.Error{Failure: provider.RequestBuildFailure("Google", err)})
			return
		}
		req.Header.Set("x-goog-api-key", r.shared.key.Expose())
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Accept", "text/event-stream")
		response, err := r.http.Do(req)
		if err != nil {
			emit(&provider.Error{Failure: provider.TransportFailure("Google", err)})
			return
		}
		if response.StatusCode < 200 || response.StatusCode >= 300 {
			failure := provider.HTTPFailure(ctx, response, "Google", provider.ReadHTTPFailure, r.shared.clock)
			emit(&provider.Error{Failure: failure})
			return
		}
		r.stream(ctx, response, emit)
	}
}

type chunk struct {
	Candidates *[]candidate   `json:"candidates"`
	Usage      *usageMetadata `json:"usageMetadata"`
	Error      *chunkError    `json:"error"`
}
type candidate struct {
	Content *candidateContent `json:"content"`
}
type candidateContent struct {
	Parts *[]responsePart `json:"parts"`
}
type responsePart struct {
	Text      *string       `json:"text"`
	Thought   *bool         `json:"thought"`
	Signature *string       `json:"thoughtSignature"`
	Call      *functionCall `json:"functionCall"`
}
type functionCall struct {
	Name provider.NonEmpty `json:"name"`
	Args *callArguments    `json:"args"`
	ID   *string           `json:"id"`
}

// callArguments uses the shared serde-compatible value reading for Gemini's tool input.
type callArguments json.RawMessage

// UnmarshalJSON reads the vendor value used by this provider.
func (a *callArguments) UnmarshalJSON(data []byte) error {
	text, err := provider.ToolArguments(data)
	if err != nil {
		return err
	}
	if bytes.HasPrefix(bytes.TrimSpace(data), []byte(`"`)) {
		encoded, err := provider.JSONBody(text)
		if err != nil {
			return err
		}
		*a = encoded
	} else {
		*a = []byte(text)
	}
	return nil
}

type usageMetadata struct {
	Prompt     *uint64 `json:"promptTokenCount"`
	Candidates *uint64 `json:"candidatesTokenCount"`
	Thoughts   *uint64 `json:"thoughtsTokenCount"`
	Cached     *uint64 `json:"cachedContentTokenCount"`
}
type chunkError struct {
	Status  provider.ReportedString `json:"status"  wire:"optional"`
	Message provider.ReportedString `json:"message" wire:"optional"`
}
type mapper struct {
	clock        core.Clock
	thinkingOpen bool
	usage        core.TokenUsage
}

func (m *mapper) frame(data string) ([]provider.Event, bool) {
	received, err := provider.DecodeUntagged[chunk](data)
	if err != nil {
		return []provider.Event{&provider.Error{Failure: provider.Undecodable("Google", err, data)}}, true
	}
	if received.Error != nil {
		message := "Google API stream error"
		if received.Error.Message.Value != nil {
			message = *received.Error.Message.Value
		}
		failure := provider.Failure{
			Message: message,
			Code:    provider.ClassifyError(received.Error.Status.Value, message),
			Diagnostics: &core.ProviderErrorDiagnostics{
				Source:       "stream",
				ProviderCode: received.Error.Status.Value,
				Upstream:     &data,
			},
		}
		return []provider.Event{
			&provider.Error{Failure: failure.WithRetryWait(provider.ReadHTTPFailure, m.clock.Now())},
		}, true
	}
	if u := received.Usage; u != nil {
		m.usage = core.TokenUsage{}
		if u.Prompt != nil {
			m.usage.InputTokens = *u.Prompt
		}
		if u.Cached != nil {
			m.usage.CacheReadTokens = *u.Cached
		}
		m.usage.InputTokens -= min(m.usage.InputTokens, m.usage.CacheReadTokens)
		if u.Candidates != nil {
			m.usage.OutputTokens = *u.Candidates
		}
		if u.Thoughts != nil {
			m.usage.OutputTokens += *u.Thoughts
		}
	}
	var out []provider.Event
	if received.Candidates != nil {
		for _, candidate := range *received.Candidates {
			if candidate.Content != nil && candidate.Content.Parts != nil {
				for _, part := range *candidate.Content.Parts {
					out = m.part(part, out)
				}
			}
		}
	}
	return out, false
}

func (m *mapper) part(part responsePart, out []provider.Event) []provider.Event {
	if call := part.Call; call != nil {
		if part.Signature != nil {
			if !m.thinkingOpen {
				out = append(out, &provider.ThinkingStart{})
			}
			out = append(out, &provider.ThinkingSignature{Signature: signatureTag + *part.Signature})
		}
		out = append(out, toolCall(call))
		m.thinkingOpen = false
		return out
	}
	if part.Thought != nil && *part.Thought {
		if !m.thinkingOpen {
			out = append(out, &provider.ThinkingStart{})
			m.thinkingOpen = true
		}
		if part.Text != nil && *part.Text != "" {
			out = append(out, &provider.ThinkingDelta{Text: *part.Text})
		}
		return out
	}
	if part.Signature != nil && m.thinkingOpen {
		out = append(out, &provider.ThinkingSignature{Signature: signatureTag + *part.Signature})
	}
	if part.Text != nil && *part.Text != "" {
		m.thinkingOpen = false
		out = append(out, &provider.TextDelta{Text: *part.Text})
	}
	return out
}

func (r *runtime) stream(ctx context.Context, response *http.Response, emit func(provider.Event) bool) {
	mapper := mapper{clock: r.shared.clock}
	for data, err := range provider.SSEData(ctx, response.Body) {
		if err != nil {
			emit(&provider.Error{Failure: provider.EventStreamFailure("Google", err)})
			return
		}
		events, ended := mapper.frame(data)
		for _, event := range events {
			if !emit(event) {
				return
			}
		}
		if ended {
			return
		}
	}
	emit(&provider.Response{Usage: mapper.usage})
}

func toolCall(call *functionCall) *provider.ToolCall {
	var id string
	if call.ID != nil {
		id = *call.ID
	} else {
		// Gemini omits call IDs. A fresh RFC 9562 UUID keeps them distinct across sessions and restarts.
		var uuid [16]byte
		_, _ = rand.Read(uuid[:]) // crypto/rand.Read fills the buffer or terminates the process.
		uuid[6] = uuid[6]&0x0f | 0x40
		uuid[8] = uuid[8]&0x3f | 0x80
		id = fmt.Sprintf("%s_%x-%x-%x-%x-%x", call.Name, uuid[:4], uuid[4:6], uuid[6:8], uuid[8:10], uuid[10:])
	}
	input := json.RawMessage(`{}`)
	if call.Args != nil {
		input = json.RawMessage(*call.Args)
	}
	return &provider.ToolCall{ToolUseID: id, ToolName: string(call.Name), Input: input}
}
