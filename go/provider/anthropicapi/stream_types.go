package anthropicapi

import (
	"encoding/json/jsontext"

	"github.com/wspl/demi/go/provider"
)

//demi:wire open
type streamTag struct {
	Type string `json:"type"`
}

//demi:wire open
type usage struct {
	InputTokens  *uint64 `json:"input_tokens,omitzero" check:"nullabsent"`
	OutputTokens *uint64 `json:"output_tokens,omitzero" check:"nullabsent"`
	CacheRead    *uint64 `json:"cache_read_input_tokens,omitzero" check:"nullabsent"`
	CacheWrite   *uint64 `json:"cache_creation_input_tokens,omitzero" check:"nullabsent"`
}

//demi:wire open
type messageStart struct {
	Message startedMessage `json:"message"`
}

//demi:wire open
type startedMessage struct {
	Usage *usage `json:"usage,omitzero" check:"nullabsent"`
}

//demi:wire open
type blockStart struct {
	Index        uint32         `json:"index"`
	ContentBlock jsontext.Value `json:"content_block"`
}

//demi:wire open
type blockDelta struct {
	Index uint32         `json:"index"`
	Delta jsontext.Value `json:"delta"`
}

//demi:wire open
type blockStop struct {
	Index uint32 `json:"index"`
}

//demi:wire open
type messageDelta struct {
	Usage *usage `json:"usage,omitzero" check:"nullabsent"`
}

//demi:wire open
type textBlock struct {
	Text string `json:"text"`
}

//demi:wire open
type redactedBlock struct {
	Data string `json:"data"`
}

//demi:wire open
type toolBlockStart struct {
	ID    string          `json:"id" check:"chars=1.."`
	Name  string          `json:"name" check:"chars=1.."`
	Input *jsontext.Value `json:"input,omitzero" check:"nullabsent"`
}

//demi:wire open
type thinkingDelta struct {
	Thinking string `json:"thinking"`
}

//demi:wire open
type signatureDelta struct {
	Signature string `json:"signature"`
}

//demi:wire open
type inputDelta struct {
	PartialJSON string `json:"partial_json"`
}

//demi:wire open
type errorEvent struct {
	Message *provider.ReportedString `json:"message,omitzero" check:"nullabsent,func=provider.Validate"`
	Error   *errorBody               `json:"error,omitzero" check:"nullabsent"`
}

//demi:wire open
type errorBody struct {
	Type    *provider.ReportedString `json:"type,omitzero" check:"nullabsent,func=provider.Validate"`
	Message *provider.ReportedString `json:"message,omitzero" check:"nullabsent,func=provider.Validate"`
}
