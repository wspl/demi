package provider

import (
	"strings"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/types"
)

// ResponsesEvent is a registered event in an OpenAI-shaped Responses stream.
//
//sumtype:decl
type ResponsesEvent interface{ responsesEvent() }

// ResponsesItemAdded is the payload of response.output_item.added.
type ResponsesItemAdded struct {
	Item *ResponsesItemJSON `json:"item"`
}

func (*ResponsesItemAdded) responsesEvent() {}

// ResponsesItemDone is the payload of response.output_item.done.
type ResponsesItemDone struct {
	Item   *ResponsesItemJSON `json:"item"`
	ItemID *string            `json:"item_id"`
	CallID *string            `json:"call_id"`
}

func (*ResponsesItemDone) responsesEvent() {}

// ResponsesTextDelta is the payload of response.output_text.delta.
type ResponsesTextDelta struct {
	Delta string `json:"delta"`
}

func (*ResponsesTextDelta) responsesEvent() {}

// ResponsesReasoningTextDelta is the payload of response.reasoning_text.delta.
type ResponsesReasoningTextDelta struct {
	Delta string `json:"delta"`
}

func (*ResponsesReasoningTextDelta) responsesEvent() {}

// ResponsesReasoningSummaryDelta is the payload of response.reasoning_summary_text.delta.
type ResponsesReasoningSummaryDelta struct {
	Delta string `json:"delta"`
}

func (*ResponsesReasoningSummaryDelta) responsesEvent() {}

// ResponsesArgumentsDelta is the payload of response.function_call_arguments.delta.
type ResponsesArgumentsDelta struct {
	ItemID *string `json:"item_id"`
	Delta  string  `json:"delta"`
}

func (*ResponsesArgumentsDelta) responsesEvent() {}

// ResponsesArgumentsDone is the payload of response.function_call_arguments.done.
type ResponsesArgumentsDone struct {
	ItemID    *string `json:"item_id"`
	Arguments string  `json:"arguments"`
}

func (*ResponsesArgumentsDone) responsesEvent() {}

// ResponsesCompleted is the payload of response.completed.
type ResponsesCompleted struct {
	Response *CompletedResponse `json:"response"`
}

func (*ResponsesCompleted) responsesEvent() {}

// ResponsesFailed is the payload of response.failed.
type ResponsesFailed struct {
	Response *FailedResponse `json:"response"`
}

func (*ResponsesFailed) responsesEvent() {}

// ResponsesIncomplete is the payload of response.incomplete.
type ResponsesIncomplete struct {
	Response *IncompleteResponse `json:"response"`
}

func (*ResponsesIncomplete) responsesEvent() {}

// ResponsesError is the payload of error.
type ResponsesError struct {
	Message ReportedString `json:"message" wire:"optional"`
	Code    ReportedString `json:"code"    wire:"optional"`
	Error   *VendorError   `json:"error"`
}

func (*ResponsesError) responsesEvent() {}

var responsePayloads = map[string]func(string) (ResponsesEvent, error){
	"response.output_item.added": func(text string) (ResponsesEvent, error) {
		value, err := DecodeUntagged[ResponsesItemAdded](text)
		return &value, err
	},
	"response.output_item.done": func(text string) (ResponsesEvent, error) {
		value, err := DecodeUntagged[ResponsesItemDone](text)
		return &value, err
	},
	"response.output_text.delta": func(text string) (ResponsesEvent, error) {
		value, err := DecodeUntagged[ResponsesTextDelta](text)
		return &value, err
	},
	"response.reasoning_text.delta": func(text string) (ResponsesEvent, error) {
		value, err := DecodeUntagged[ResponsesReasoningTextDelta](text)
		return &value, err
	},
	"response.reasoning_summary_text.delta": func(text string) (ResponsesEvent, error) {
		value, err := DecodeUntagged[ResponsesReasoningSummaryDelta](text)
		return &value, err
	},
	"response.function_call_arguments.delta": func(text string) (ResponsesEvent, error) {
		value, err := DecodeUntagged[ResponsesArgumentsDelta](text)
		return &value, err
	},
	"response.function_call_arguments.done": func(text string) (ResponsesEvent, error) {
		value, err := DecodeUntagged[ResponsesArgumentsDone](text)
		return &value, err
	},
	"response.completed": func(text string) (ResponsesEvent, error) {
		value, err := DecodeUntagged[ResponsesCompleted](text)
		return &value, err
	},
	"response.failed": func(text string) (ResponsesEvent, error) {
		value, err := DecodeUntagged[ResponsesFailed](text)
		return &value, err
	},
	"response.incomplete": func(text string) (ResponsesEvent, error) {
		value, err := DecodeUntagged[ResponsesIncomplete](text)
		return &value, err
	},
	"error": func(text string) (ResponsesEvent, error) {
		value, err := DecodeUntagged[ResponsesError](text)
		return &value, err
	},
}

// Received is a decoded event with the exact text it arrived as.
type Received struct {
	Event ResponsesEvent
	Text  string
}

// DecodeResponsesFrame skips DONE and unknown tags (ok is false), but refuses malformed known payloads.
func DecodeResponsesFrame(data string) (Received, bool, error) {
	if strings.TrimSpace(data) == "[DONE]" {
		return Received{}, false, nil
	}
	event, ok, err := DecodeTagged(data, responsePayloads)
	if err != nil || !ok {
		return Received{}, false, err
	}
	return Received{Event: event, Text: data}, true, nil
}

// CompletedResponse holds the final API usage.
type CompletedResponse struct {
	Usage *ResponsesUsage `json:"usage"`
}

// ResponsesUsage holds whole-number token counts including cache reads and writes.
type ResponsesUsage struct {
	InputTokens        *uint64             `json:"input_tokens"`
	OutputTokens       *uint64             `json:"output_tokens"`
	InputTokensDetails *InputTokensDetails `json:"input_tokens_details"`
}

// InputTokensDetails holds cached input token counts.
type InputTokensDetails struct {
	CachedTokens     *uint64 `json:"cached_tokens"`
	CacheWriteTokens *uint64 `json:"cache_write_tokens"`
}

// TokenUsage separates both cache counts from input tokens.
func (u ResponsesUsage) TokenUsage() types.TokenUsage {
	var read, written *uint64
	if u.InputTokensDetails != nil {
		read = u.InputTokensDetails.CachedTokens
		written = u.InputTokensDetails.CacheWriteTokens
	}
	return usageWithCachedInput(u.InputTokens, u.OutputTokens, read, written)
}

// FailedResponse reports a response ID and error when the vendor supplies them.
type FailedResponse struct {
	ID    ReportedString `json:"id"    wire:"optional"`
	Error *VendorError   `json:"error"`
}

// IncompleteResponse reports why a response stopped early.
type IncompleteResponse struct {
	IncompleteDetails *IncompleteDetails `json:"incomplete_details"`
}

// IncompleteDetails carries a reported reason.
type IncompleteDetails struct {
	Reason ReportedString `json:"reason" wire:"optional"`
}

// VendorError is the failure object shared by Responses errors and failed responses.
type VendorError struct {
	Code           ReportedString `json:"code"       wire:"optional"`
	Kind           ReportedString `json:"type"       wire:"optional"`
	Message        ReportedString `json:"message"    wire:"optional"`
	RequestID      ReportedString `json:"request_id" wire:"optional"`
	RequestIDCamel ReportedString `json:"requestId"  wire:"optional"`
}

// ResponsesItem is a registered output item; unknown kinds are skipped.
//
//sumtype:decl
type ResponsesItem interface{ responsesItem() }

// ResponsesItemJSON decodes a nested tagged output item.
type ResponsesItemJSON struct{ Value ResponsesItem }

// UnmarshalJSON decodes known item types and skips future types.
func (r *ResponsesItemJSON) UnmarshalJSON(data []byte) error {
	canonical, err := canonicalVendorJSON(data, 0)
	if err != nil {
		return err
	}
	value, _, err := DecodeTagged(string(canonical), itemPayloads)
	r.Value = value
	return err
}

var itemPayloads = map[string]func(string) (ResponsesItem, error){
	"reasoning": func(text string) (ResponsesItem, error) {
		value, err := DecodeUntagged[ReasoningItem](text)
		return &value, err
	},
	"message": func(text string) (ResponsesItem, error) {
		value, err := DecodeUntagged[MessageItem](text)
		return &value, err
	},
	"function_call": func(text string) (ResponsesItem, error) {
		value, err := DecodeUntagged[FunctionCallItem](text)
		return &value, err
	},
}

// ReasoningItem retains all vendor fields for signed reasoning replay.
type ReasoningItem struct {
	Kind             string           `json:"type"`
	ID               *string          `json:"id,omitempty"`
	Summary          *[]ReasoningPart `json:"summary,omitempty"`
	Content          *[]ReasoningPart `json:"content,omitempty"`
	EncryptedContent *string          `json:"encrypted_content,omitempty"`
	Extra            []contract.Field `json:"-"`
}

func (*ReasoningItem) responsesItem() {}

// UnmarshalJSON retains reasoning fields in received order for replay.
func (r *ReasoningItem) UnmarshalJSON(data []byte) error {
	type plain ReasoningItem
	value, err := DecodeUntagged[plain](string(data))
	if err != nil {
		return err
	}
	if value.Kind != "reasoning" {
		return &WireError{Field: "type", Err: errReasoningType}
	}
	extra, err := reasoningExtras(data, "type", "id", "summary", "content", "encrypted_content")
	if err != nil {
		return err
	}
	value.Extra = extra
	*r = ReasoningItem(value)
	return nil
}

// MarshalJSON encodes reasoning without changing extra-field order.
func (r ReasoningItem) MarshalJSON() ([]byte, error) {
	fields := []contract.Field{{Name: "type", Value: r.Kind}}
	if r.ID != nil {
		fields = append(fields, contract.Field{Name: "id", Value: *r.ID})
	}
	if r.Summary != nil {
		fields = append(fields, contract.Field{Name: "summary", Value: *r.Summary})
	}
	if r.Content != nil {
		fields = append(fields, contract.Field{Name: "content", Value: *r.Content})
	}
	if r.EncryptedContent != nil {
		fields = append(fields, contract.Field{Name: "encrypted_content", Value: *r.EncryptedContent})
	}
	fields = append(fields, r.Extra...)
	return contract.EncodeObject(fields)
}

// Text returns the summary, falling back to content when the summary is empty.
func (r ReasoningItem) Text() string {
	join := func(parts *[]ReasoningPart) string {
		if parts == nil {
			return ""
		}
		texts := make([]string, len(*parts))
		for i, part := range *parts {
			texts[i] = part.Text
		}
		return strings.Join(texts, "\n\n")
	}
	if summary := join(r.Summary); summary != "" {
		return summary
	}
	return join(r.Content)
}

// ReasoningPart retains a reasoning text part and any fields not read by Demi.
type ReasoningPart struct {
	Text  string           `json:"text"`
	Extra []contract.Field `json:"-"`
}

// UnmarshalJSON retains summary fields in received order for replay.
func (r *ReasoningPart) UnmarshalJSON(data []byte) error {
	type plain ReasoningPart
	value, err := DecodeUntagged[plain](string(data))
	if err != nil {
		return err
	}
	extra, err := reasoningExtras(data, "text")
	if err != nil {
		return err
	}
	*r = ReasoningPart{Text: value.Text, Extra: extra}
	return nil
}

// MarshalJSON preserves the fields of a reasoning summary.
func (r ReasoningPart) MarshalJSON() ([]byte, error) {
	fields := []contract.Field{{Name: "text", Value: r.Text}}
	return contract.EncodeObject(append(fields, r.Extra...))
}

// MessageItem carries output text and refusal parts.
type MessageItem struct {
	Content *[]MessagePartJSON `json:"content"`
}

func (*MessageItem) responsesItem() {}

// Text joins output text and refusals without a separator.
func (m MessageItem) Text() string {
	var out strings.Builder
	if m.Content != nil {
		for _, part := range *m.Content {
			switch p := part.Value.(type) {
			case *MessageOutputText:
				out.WriteString(p.Text)
			case *MessageRefusal:
				out.WriteString(p.Refusal)
			}
		}
	}
	return out.String()
}

// MessagePart is a registered content part of a Responses message.
//
//sumtype:decl
type MessagePart interface{ messagePart() }

// MessageOutputText is a message's output text.
type MessageOutputText struct {
	Text string `json:"text"`
}

// MessageRefusal is a message's refusal text.
type MessageRefusal struct {
	Refusal string `json:"refusal"`
}

func (*MessageOutputText) messagePart() {}
func (*MessageRefusal) messagePart()    {}

// MessagePartJSON decodes a nested tagged content part.
type MessagePartJSON struct{ Value MessagePart }

// UnmarshalJSON decodes text and refusal parts and skips future types.
func (p *MessagePartJSON) UnmarshalJSON(data []byte) error {
	canonical, err := canonicalVendorJSON(data, 0)
	if err != nil {
		return err
	}
	value, _, err := DecodeTagged(string(canonical), messagePayloads)
	p.Value = value
	return err
}

var messagePayloads = map[string]func(string) (MessagePart, error){
	"output_text": func(text string) (MessagePart, error) {
		value, err := DecodeUntagged[MessageOutputText](text)
		return &value, err
	},
	"refusal": func(text string) (MessagePart, error) {
		value, err := DecodeUntagged[MessageRefusal](text)
		return &value, err
	},
}

// FunctionCallItem describes a completed or newly opened function call.
type FunctionCallItem struct {
	ID        *string `json:"id"`
	CallID    *string `json:"call_id"`
	Name      *string `json:"name"`
	Arguments *string `json:"arguments"`
}

func (*FunctionCallItem) responsesItem() {}

// ToolUseID combines the call and item identifiers needed for replay.
func ToolUseID(callID, itemID string) string {
	return callID + "|" + itemID
}

// SplitToolUseID separates a call ID from its optional item ID.
func SplitToolUseID(id string) (string, *string) {
	call, item, ok := strings.Cut(id, "|")
	if !ok {
		return call, nil
	}
	return call, &item
}
