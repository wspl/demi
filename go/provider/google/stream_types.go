package google

import (
	"encoding/json/jsontext"

	"github.com/wspl/demi/go/provider"
)

//demi:wire open
type chunk struct {
	Candidates *[]candidate   `json:"candidates,omitzero" check:"nullabsent"`
	Usage      *usageMetadata `json:"usageMetadata,omitzero" check:"nullabsent"`
	Error      *chunkError    `json:"error,omitzero" check:"nullabsent"`
}

//demi:wire open
type candidate struct {
	Content *candidateContent `json:"content,omitzero" check:"nullabsent"`
}

//demi:wire open
type candidateContent struct {
	Parts *[]responsePart `json:"parts,omitzero" check:"nullabsent"`
}

//demi:wire open
type responsePart struct {
	Text             *string           `json:"text,omitzero" check:"nullabsent"`
	Thought          *bool             `json:"thought,omitzero" check:"nullabsent"`
	ThoughtSignature *string           `json:"thoughtSignature,omitzero" check:"nullabsent"`
	FunctionCall     *functionCallPart `json:"functionCall,omitzero" check:"nullabsent"`
}

//demi:wire open
type functionCallPart struct {
	Name string          `json:"name" check:"chars=1.."`
	Args *jsontext.Value `json:"args,omitzero" check:"nullabsent"`
	ID   *string         `json:"id,omitzero" check:"nullabsent"`
}

//demi:wire open
type usageMetadata struct {
	Prompt     *uint64 `json:"promptTokenCount,omitzero" check:"nullabsent"`
	Candidates *uint64 `json:"candidatesTokenCount,omitzero" check:"nullabsent"`
	Thoughts   *uint64 `json:"thoughtsTokenCount,omitzero" check:"nullabsent"`
	Cached     *uint64 `json:"cachedContentTokenCount,omitzero" check:"nullabsent"`
}

//demi:wire open
type chunkError struct {
	Status  *provider.ReportedString `json:"status,omitzero" check:"nullabsent,func=provider.Validate"`
	Message *provider.ReportedString `json:"message,omitzero" check:"nullabsent,func=provider.Validate"`
}
