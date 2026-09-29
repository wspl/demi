package provider

import (
	"context"
	"encoding/json/v2"
	"fmt"
	"io"
	"iter"
	"regexp"
	"strings"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/rustfmt"
	"github.com/wspl/demi/go/internal/wire"
)

type ResponsesMapper struct {
	vendor                          Vendor
	signatureTag                    string
	currentCall                     *string
	arguments                       map[string]string
	reasoningStreamed, textStreamed bool
}

func NewResponsesMapper(vendor Vendor, signatureTag string) *ResponsesMapper {
	return &ResponsesMapper{vendor: vendor, signatureTag: signatureTag, arguments: make(map[string]string)}
}
func (m *ResponsesMapper) Frame(data string) ([]ProviderEvent, bool) {
	return m.FrameWithRecord(data, data)
}

// FrameWithRecord preserves a transport envelope as the failure record.
func (m *ResponsesMapper) FrameWithRecord(data, record string) ([]ProviderEvent, bool) {
	events, ended, err := m.frame(data, record)
	if err != nil {
		return []ProviderEvent{FailureEvent{Failure: Undecodable(m.vendor.Label, err, record)}}, true
	}
	return events, ended
}

// FrameChecked separates malformed protocol input from a vendor-reported failure.
func (m *ResponsesMapper) FrameChecked(data, record string) ([]ProviderEvent, bool, error) {
	return m.frame(data, record)
}

func (m *ResponsesMapper) frame(data, record string) ([]ProviderEvent, bool, error) {
	if strings.TrimSpace(data) == "[DONE]" {
		return nil, false, nil
	}
	tag, err := decode[vendorTag]([]byte(data))
	if err != nil {
		return nil, true, err
	}
	switch tag.Type {
	case "response.output_item.added", "response.output_item.done":
		event, err := decode[ResponsesItemEvent]([]byte(data))
		if err != nil {
			return nil, true, err
		}
		if event.Item == nil {
			return nil, false, nil
		}
		events, err := m.item(event, tag.Type == "response.output_item.done")
		return events, false, err
	case "response.output_text.delta", "response.reasoning_text.delta", "response.reasoning_summary_text.delta", "response.function_call_arguments.delta":
		event, err := decode[ResponsesDelta]([]byte(data))
		if err != nil {
			return nil, true, err
		}
		if tag.Type == "response.function_call_arguments.delta" {
			id := event.ItemID
			if id == nil {
				id = m.currentCall
			}
			if id != nil {
				m.arguments[*id] += event.Delta
			}
			return nil, false, nil
		}
		if tag.Type == "response.output_text.delta" {
			m.textStreamed = true
			if event.Delta != "" {
				return []ProviderEvent{TextDelta{Text: event.Delta}}, false, nil
			}
		} else {
			m.reasoningStreamed = true
			if event.Delta != "" {
				return []ProviderEvent{ThinkingDelta{Text: event.Delta}}, false, nil
			}
		}
	case "response.function_call_arguments.done":
		event, err := decode[ResponsesArgumentsDone]([]byte(data))
		if err != nil {
			return nil, true, err
		}
		id := event.ItemID
		if id == nil {
			id = m.currentCall
		}
		if id != nil {
			m.arguments[*id] = event.Arguments
		}
	case "response.completed":
		event, err := decode[ResponsesCompleted]([]byte(data))
		if err != nil {
			return nil, true, err
		}
		usage := core.TokenUsage{}
		if event.Response != nil && event.Response.Usage != nil {
			usage = event.Response.Usage.TokenUsage()
		}
		return []ProviderEvent{Response{Usage: usage}}, true, nil
	case "response.failed":
		event, err := decode[ResponsesFailed]([]byte(data))
		if err != nil {
			return nil, true, err
		}
		var vendorError *ResponsesVendorError
		var responseID *string
		if event.Response != nil {
			vendorError = event.Response.Error
			responseID = reported(event.Response.ID)
		}
		return []ProviderEvent{FailureEvent{Failure: m.failure(vendorError, nil, nil, m.vendor.Label+" response failed", responseID, record)}}, true, nil
	case "response.incomplete":
		event, err := decode[ResponsesIncomplete]([]byte(data))
		if err != nil {
			return nil, true, err
		}
		reason := "unknown"
		if event.Response != nil && event.Response.IncompleteDetails != nil {
			if value := reported(event.Response.IncompleteDetails.Reason); value != nil {
				reason = *value
			}
		}
		code := Incomplete
		if reason == "max_output_tokens" {
			code = ContextLengthExceeded
		}
		failure := ProviderFailure{Message: fmt.Sprintf("Incomplete %s response returned, reason: %s", m.vendor.Label, reason), Code: code, Diagnostics: &core.ProviderErrorDiagnostics{Source: core.FailureSourceStream, Upstream: &record}}
		return []ProviderEvent{FailureEvent{Failure: failure}}, true, nil
	case "error":
		event, err := decode[ResponsesError]([]byte(data))
		if err != nil {
			return nil, true, err
		}
		return []ProviderEvent{FailureEvent{Failure: m.failure(event.Error, reported(event.Message), reported(event.Code), m.vendor.Label+" stream error", nil, record)}}, true, nil
	}
	return nil, false, nil
}
func (m *ResponsesMapper) item(event ResponsesItemEvent, done bool) ([]ProviderEvent, error) {
	raw := []byte(*event.Item)
	tag, err := decode[vendorTag](raw)
	if err != nil {
		return nil, wire.In("item", err)
	}
	var events []ProviderEvent
	switch tag.Type {
	case "reasoning":
		item, err := decode[ReasoningItem](raw)
		if err != nil {
			return nil, wire.In("item", err)
		}
		if !done {
			return []ProviderEvent{ThinkingStart{}}, nil
		}
		if !m.reasoningStreamed {
			if text := item.Text(); text != "" {
				events = append(events, ThinkingDelta{Text: text})
			}
		}
		encoded, err := json.Marshal(item, json.Deterministic(true))
		if err != nil {
			return nil, err
		}
		// The Rust writes the item from the values it read, so its members
		// keep their order and their numbers and strings are spelled as
		// serde_json prints them (1e0 is 1.0).
		encoded, err = rustfmt.NormalizeJSON(encoded)
		if err != nil {
			return nil, wire.In("item", err)
		}
		events = append(events, ThinkingSignature{Signature: m.signatureTag + string(encoded)})
		m.reasoningStreamed = false
	case "message":
		item, err := decode[ResponsesMessage](raw)
		if err != nil {
			return nil, wire.In("item", err)
		}
		text := ""
		if item.Content != nil {
			for i, raw := range *item.Content {
				part, err := decode[vendorTag](raw)
				if err != nil {
					return nil, wire.In("item.content", wire.In(wire.Index(i), err))
				}
				switch part.Type {
				case "output_text":
					value, err := decode[ResponsesOutputText](raw)
					if err != nil {
						return nil, wire.In("item.content", wire.In(wire.Index(i), err))
					}
					text += value.Text
				case "refusal":
					value, err := decode[ResponsesRefusal](raw)
					if err != nil {
						return nil, wire.In("item.content", wire.In(wire.Index(i), err))
					}
					text += value.Refusal
				}
			}
		}
		if done {
			if !m.textStreamed && text != "" {
				events = append(events, TextDelta{Text: text})
			}
			m.textStreamed = false
		}
	case "function_call":
		call, err := decode[ResponsesFunctionCall](raw)
		if err != nil {
			return nil, wire.In("item", err)
		}
		if !done {
			if call.ID != nil {
				arguments := ""
				if call.Arguments != nil {
					arguments = *call.Arguments
				}
				m.arguments[*call.ID] = arguments
				m.currentCall = call.ID
			}
			return nil, nil
		}
		itemID, callID := call.ID, call.CallID
		if itemID == nil {
			itemID = event.ItemID
		}
		if callID == nil {
			callID = event.CallID
		}
		if itemID != nil && callID != nil && call.Name != nil {
			arguments, ok := m.arguments[*itemID]
			if !ok {
				arguments = "{}"
				if call.Arguments != nil {
					arguments = *call.Arguments
				}
			}
			events = append(events, ToolCall{ToolUseID: ToolUseID(*callID, *itemID), ToolName: *call.Name, Input: ToolInput(arguments)})
		}
		if itemID != nil {
			delete(m.arguments, *itemID)
			if m.currentCall != nil && *m.currentCall == *itemID {
				m.currentCall = nil
			}
		}
	}
	return events, nil
}

var responseRequestID = regexp.MustCompile(`(?i)request ID ([A-Za-z0-9-]+)`)

func (m *ResponsesMapper) failure(vendorError *ResponsesVendorError, message, code *string, fallback string, responseID *string, text string) ProviderFailure {
	var requestID *string
	if vendorError != nil {
		if message == nil {
			message = reported(vendorError.Message)
		}
		if code == nil {
			code = reported(vendorError.Code)
		}
		if code == nil {
			code = reported(vendorError.Type)
		}
		requestID = reported(vendorError.RequestID)
		if requestID == nil {
			requestID = reported(vendorError.RequestIDCamel)
		}
	}
	if message == nil {
		message = &fallback
	}
	if requestID == nil {
		if matches := responseRequestID.FindStringSubmatch(*message); matches != nil {
			requestID = &matches[1]
		}
	}
	codeName := ""
	if code != nil {
		codeName = *code
	}
	failure := ProviderFailure{Message: *message, Code: ClassifyVendorFailure(codeName, *message), Diagnostics: &core.ProviderErrorDiagnostics{Source: core.FailureSourceStream, ProviderCode: code, ProviderRequestID: requestID, ProviderResponseID: responseID, Upstream: &text}}
	return failure.WithRetryWait(m.vendor.Reader, m.vendor.Clock.Now())
}
func MapResponsesSSE(ctx context.Context, body io.Reader, vendor Vendor, signatureTag string) iter.Seq[ProviderEvent] {
	return func(yield func(ProviderEvent) bool) {
		mapper := NewResponsesMapper(vendor, signatureTag)
		for data, err := range SSEData(body) {
			if ctx.Err() != nil {
				return
			}
			if err != nil {
				yield(FailureEvent{Failure: EventStreamFailure(vendor.Label, err)})
				return
			}
			events, ended := mapper.Frame(data)
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
			yield(Response{})
		}
	}
}

// ResponsesEventKnown identifies the registered events, before a transport
// decides that its fallback window has ended. Payload validation follows in Frame.
func ResponsesEventKnown(data []byte) (bool, error) {
	tag, err := decode[vendorTag](data)
	if err != nil {
		return false, err
	}
	switch tag.Type {
	case "response.output_item.added", "response.output_item.done", "response.output_text.delta", "response.reasoning_text.delta", "response.reasoning_summary_text.delta", "response.function_call_arguments.delta", "response.function_call_arguments.done", "response.completed", "response.failed", "response.incomplete", "error":
		return true, nil
	}
	return false, nil
}
