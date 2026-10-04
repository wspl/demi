package provider

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"iter"
	"regexp"
	"slices"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/types"
)

var errReasoningType = errors.New("expected reasoning type")

// reasoningExtras preserves the order of unknown fields for signed replay.
func reasoningExtras(data []byte, known ...string) ([]contract.Field, error) {
	fields := make([]contract.Field, 0)
	err := vendorMembers(data, func(name string, raw json.RawMessage) error {
		if slices.Contains(known, name) {
			return nil
		}
		value, err := canonicalVendorJSON(raw, 0)
		if err != nil {
			return err
		}
		for i := range fields {
			if fields[i].Name == name {
				fields[i].Value = value
				return nil
			}
		}
		fields = append(fields, contract.Field{Name: name, Value: value})
		return nil
	})
	return fields, err
}

// ResponsesSSEEvents decodes registered events with their received text and closes
// the body when iteration ends. A malformed frame is the final item.
func ResponsesSSEEvents(ctx context.Context, body io.ReadCloser, label string) iter.Seq2[Received, error] {
	return func(yield func(Received, error) bool) {
		for data, err := range SSEData(ctx, body) {
			if err != nil {
				failure := EventStreamFailure(label, err)
				yield(Received{}, &failure)
				return
			}
			received, ok, err := DecodeResponsesFrame(data)
			if err != nil {
				failure := Undecodable(label, err, data)
				yield(Received{}, &failure)
				return
			}
			if ok && !yield(received, nil) {
				return
			}
		}
	}
}

// MapResponsesEvents maps an SSE or WebSocket event iterator to a run. A clean
// end without a completion event produces zero usage; cancellation produces none.
func MapResponsesEvents(
	ctx context.Context,
	events iter.Seq2[Received, error],
	vendor Vendor,
	signatureTag string,
) Run {
	return func(yield func(Event) bool) {
		mapper := responsesMapper{vendor: vendor, signatureTag: signatureTag, arguments: make(map[string]string)}
		for received, err := range events {
			if ctx.Err() != nil {
				return
			}
			if err != nil {
				var failure *Failure
				if !errors.As(err, &failure) {
					value := NoAnswer(err.Error())
					failure = &value
				}
				yield(&Error{Failure: *failure})
				return
			}
			out, ended := mapper.event(received)
			for _, event := range out {
				if ctx.Err() != nil || !yield(event) {
					return
				}
			}
			if ended {
				return
			}
		}
		if ctx.Err() == nil {
			yield(&Response{})
		}
	}
}

type responsesMapper struct {
	vendor            Vendor
	signatureTag      string
	currentCall       *string
	arguments         map[string]string
	reasoningStreamed bool
	textStreamed      bool
}

func (m *responsesMapper) event(received Received) ([]Event, bool) {
	var out []Event
	switch event := received.Event.(type) {
	case *ResponsesItemAdded:
		out = append(out, m.itemAdded(event)...)
	case *ResponsesReasoningTextDelta:
		m.reasoningStreamed = true
		if event.Delta != "" {
			out = append(out, &ThinkingDelta{Text: event.Delta})
		}
	case *ResponsesReasoningSummaryDelta:
		m.reasoningStreamed = true
		if event.Delta != "" {
			out = append(out, &ThinkingDelta{Text: event.Delta})
		}
	case *ResponsesTextDelta:
		m.textStreamed = true
		if event.Delta != "" {
			out = append(out, &TextDelta{Text: event.Delta})
		}
	case *ResponsesArgumentsDelta:
		m.argumentsDelta(event)
	case *ResponsesArgumentsDone:
		m.argumentsDone(event)
	case *ResponsesItemDone:
		out = append(out, m.itemDone(event)...)
	case *ResponsesCompleted:
		return completedResponse(event)
	case *ResponsesFailed:
		var responseID *string
		var vendorError *VendorError
		if event.Response != nil {
			responseID = event.Response.ID.Value
			vendorError = event.Response.Error
		}
		failure := m.failure(vendorError, nil, nil, m.vendor.Label+" response failed", responseID, received.Text)
		return []Event{&Error{Failure: failure}}, true
	case *ResponsesIncomplete:
		return m.incomplete(event, received)
	case *ResponsesError:
		failure := m.failure(
			event.Error,
			event.Message.Value,
			event.Code.Value,
			m.vendor.Label+
				" stream error",
			nil,
			received.Text,
		)
		return []Event{&Error{Failure: failure}}, true
	}
	return out, false
}

func (m *responsesMapper) itemDone(done *ResponsesItemDone) []Event {
	var out []Event
	if done.Item == nil {
		return out
	}
	switch item := done.Item.Value.(type) {
	case *ReasoningItem:
		if !m.reasoningStreamed {
			if text := item.Text(); text != "" {
				out = append(out, &ThinkingDelta{Text: text})
			}
		}
		encoded, err := contract.EncodeJSON(item)
		if err != nil {
			return []Event{&Error{Failure: ProtocolFailure("reasoning item cannot be encoded", "")}}
		}
		out = append(out, &ThinkingSignature{Signature: m.signatureTag + string(encoded)})
		m.reasoningStreamed = false
	case *MessageItem:
		if !m.textStreamed {
			if text := item.Text(); text != "" {
				out = append(out, &TextDelta{Text: text})
			}
		}
		m.textStreamed = false
	case *FunctionCallItem:
		out = append(out, m.callDone(item, done)...)
	}
	return out
}

var requestIDPattern = regexp.MustCompile(`(?i)request ID ([A-Za-z0-9-]+)`)

func (m *responsesMapper) failure(
	vendorError *VendorError,
	message, code *string,
	fallback string,
	responseID *string,
	text string,
) Failure {
	var requestID *string
	if vendorError != nil {
		if message == nil {
			message = vendorError.Message.Value
		}
		if code == nil {
			code = vendorError.Code.Value
		}
		if code == nil {
			code = vendorError.Kind.Value
		}
		requestID = vendorError.RequestID.Value
		if requestID == nil {
			requestID = vendorError.RequestIDCamel.Value
		}
	}
	if message == nil {
		message = &fallback
	}
	if requestID == nil {
		if match := requestIDPattern.FindStringSubmatch(*message); match != nil {
			requestID = &match[1]
		}
	}
	failure := Failure{
		Message: *message,
		Code:    ClassifyError(code, *message),
		Diagnostics: &types.ProviderErrorDiagnostics{
			Source:             "stream",
			ProviderCode:       code,
			ProviderRequestID:  requestID,
			ProviderResponseID: responseID,
			Upstream:           &text,
		},
	}
	return failure.WithRetryWait(m.vendor.Reader, m.vendor.Clock.Now())
}

func (m *responsesMapper) itemAdded(event *ResponsesItemAdded) []Event {
	var out []Event
	if event.Item != nil {
		switch item := event.Item.Value.(type) {
		case *ReasoningItem:
			out = append(out, &ThinkingStart{})
		case *FunctionCallItem:
			if item.ID != nil {
				arguments := ""
				if item.Arguments != nil {
					arguments = *item.Arguments
				}
				m.arguments[*item.ID] = arguments
				m.currentCall = item.ID
			}
		case *MessageItem:
		}
	}
	return out
}

func (m *responsesMapper) incomplete(event *ResponsesIncomplete, received Received) ([]Event, bool) {
	reason := "unknown"
	if response := event.Response; response != nil && response.IncompleteDetails != nil &&
		response.IncompleteDetails.Reason.Value != nil {
		reason = *response.IncompleteDetails.Reason.Value
	}
	code := Incomplete
	if reason == "max_output_tokens" {
		code = ContextLengthExceeded
	}
	failure := Failure{
		Message: "Incomplete " +
			m.vendor.Label +
			" response returned, reason: " +
			reason,
		Code:        &code,
		Diagnostics: &types.ProviderErrorDiagnostics{Source: "stream", Upstream: &received.Text},
	}
	return []Event{&Error{Failure: failure}}, true
}

func (m *responsesMapper) callDone(item *FunctionCallItem, done *ResponsesItemDone) []Event {
	var out []Event
	itemID := item.ID
	if itemID == nil {
		itemID = done.ItemID
	}
	callID := item.CallID
	if callID == nil {
		callID = done.CallID
	}
	if itemID != nil && callID != nil && item.Name != nil {
		arguments, ok := m.arguments[*itemID]
		if !ok {
			arguments = "{}"
			if item.Arguments != nil {
				arguments = *item.Arguments
			}
		}
		out = append(
			out,
			&ToolCall{ToolUseID: ToolUseID(*callID, *itemID), ToolName: *item.Name, Input: toolInput(arguments)},
		)
	}
	if itemID != nil {
		delete(m.arguments, *itemID)
		if m.currentCall != nil && *m.currentCall == *itemID {
			m.currentCall = nil
		}
	}
	return out
}

func (m *responsesMapper) argumentsDelta(event *ResponsesArgumentsDelta) {
	id := event.ItemID
	if id == nil {
		id = m.currentCall
	}
	if id != nil {
		m.arguments[*id] += event.Delta
	}
}

func (m *responsesMapper) argumentsDone(event *ResponsesArgumentsDone) {
	id := event.ItemID
	if id == nil {
		id = m.currentCall
	}
	if id != nil {
		m.arguments[*id] = event.Arguments
	}
}

func completedResponse(event *ResponsesCompleted) ([]Event, bool) {
	var usage types.TokenUsage
	if event.Response != nil && event.Response.Usage != nil {
		usage = event.Response.Usage.TokenUsage()
	}
	return []Event{&Response{Usage: usage}}, true
}
