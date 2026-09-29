package provider

import (
	"encoding/json/jsontext"
	"strings"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/wire"
)

//demi:wire open
type ResponsesItemEvent struct {
	Item   *jsontext.Value `json:"item,omitzero" check:"nullabsent"`
	ItemID *string         `json:"item_id,omitzero" check:"nullabsent"`
	CallID *string         `json:"call_id,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesDelta struct {
	Delta  string  `json:"delta"`
	ItemID *string `json:"item_id,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesArgumentsDone struct {
	Arguments string  `json:"arguments"`
	ItemID    *string `json:"item_id,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesCompleted struct {
	Response *ResponsesCompletedResponse `json:"response,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesCompletedResponse struct {
	Usage *ResponsesUsage `json:"usage,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesUsage struct {
	InputTokens        *uint64                `json:"input_tokens,omitzero" check:"nullabsent"`
	OutputTokens       *uint64                `json:"output_tokens,omitzero" check:"nullabsent"`
	InputTokensDetails *ResponsesInputDetails `json:"input_tokens_details,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesInputDetails struct {
	CachedTokens     *uint64 `json:"cached_tokens,omitzero" check:"nullabsent"`
	CacheWriteTokens *uint64 `json:"cache_write_tokens,omitzero" check:"nullabsent"`
}

func (u ResponsesUsage) TokenUsage() core.TokenUsage {
	var read, written *uint64
	if u.InputTokensDetails != nil {
		read = u.InputTokensDetails.CachedTokens
		written = u.InputTokensDetails.CacheWriteTokens
	}
	return UsageWithCachedInput(u.InputTokens, u.OutputTokens, read, written)
}

//demi:wire open
type ResponsesFailed struct {
	Response *ResponsesFailedResponse `json:"response,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesFailedResponse struct {
	ID    *ReportedString       `json:"id,omitzero" check:"nullabsent"`
	Error *ResponsesVendorError `json:"error,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesIncomplete struct {
	Response *ResponsesIncompleteResponse `json:"response,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesIncompleteResponse struct {
	IncompleteDetails *ResponsesIncompleteDetails `json:"incomplete_details,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesIncompleteDetails struct {
	Reason *ReportedString `json:"reason,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesError struct {
	Message *ReportedString       `json:"message,omitzero" check:"nullabsent"`
	Code    *ReportedString       `json:"code,omitzero" check:"nullabsent"`
	Error   *ResponsesVendorError `json:"error,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesVendorError struct {
	Code           *ReportedString `json:"code,omitzero" check:"nullabsent"`
	Type           *ReportedString `json:"type,omitzero" check:"nullabsent"`
	Message        *ReportedString `json:"message,omitzero" check:"nullabsent"`
	RequestID      *ReportedString `json:"request_id,omitzero" check:"nullabsent"`
	RequestIDCamel *ReportedString `json:"requestId,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesMessage struct {
	Content *[]jsontext.Value `json:"content,omitzero" check:"nullabsent"`
}

//demi:wire open
type ResponsesOutputText struct {
	Text string `json:"text"`
}

//demi:wire open
type ResponsesRefusal struct {
	Refusal string `json:"refusal"`
}

//demi:wire open
type ResponsesFunctionCall struct {
	ID        *string `json:"id,omitzero" check:"nullabsent"`
	CallID    *string `json:"call_id,omitzero" check:"nullabsent"`
	Name      *string `json:"name,omitzero" check:"nullabsent"`
	Arguments *string `json:"arguments,omitzero" check:"nullabsent"`
}

//demi:wire
type ReasoningItem struct {
	Type             string           `json:"type" check:"eq=reasoning"`
	ID               *string          `json:"id,omitzero" check:"nullabsent"`
	Summary          *[]ReasoningPart `json:"summary,omitzero" check:"nullabsent"`
	Content          *[]ReasoningPart `json:"content,omitzero" check:"nullabsent"`
	EncryptedContent *string          `json:"encrypted_content,omitzero" check:"nullabsent"`
	Extra            wire.Members     `json:",inline"`
}

//demi:wire
type ReasoningPart struct {
	Text  string       `json:"text"`
	Extra wire.Members `json:",inline"`
}

func (r ReasoningItem) Text() string {
	join := func(parts *[]ReasoningPart) string {
		if parts == nil {
			return ""
		}
		texts := make([]string, 0, len(*parts))
		for _, part := range *parts {
			texts = append(texts, part.Text)
		}
		return strings.Join(texts, "\n\n")
	}
	summary := join(r.Summary)
	if summary != "" {
		return summary
	}
	return join(r.Content)
}
func ToolUseID(callID, itemID string) string { return callID + "|" + itemID }
func SplitToolUseID(id string) (string, *string) {
	call, item, ok := strings.Cut(id, "|")
	if !ok {
		return call, nil
	}
	return call, &item
}
