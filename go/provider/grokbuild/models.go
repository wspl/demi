package grokbuild

import (
	"context"
	"encoding/json/v2"
	"fmt"
	"net/http"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

//demi:wire open
type modelsEnvelope struct {
	Data *[]model `json:"data,omitzero" check:"nullabsent"`
}

//demi:wire open
type model struct {
	ID                      *string   `json:"id,omitzero" check:"nullabsent,chars=1.."`
	Model                   *string   `json:"model,omitzero" check:"nullabsent,chars=1.."`
	Name                    *string   `json:"name,omitzero" check:"nullabsent,chars=1.."`
	Description             *string   `json:"description,omitzero" check:"nullabsent"`
	ContextWindow           *uint32   `json:"context_window,omitzero" check:"nullabsent,range=1.."`
	SupportsReasoningEffort *bool     `json:"supports_reasoning_effort,omitzero" check:"nullabsent"`
	ReasoningEffort         *string   `json:"reasoning_effort,omitzero" check:"nullabsent,chars=1.."`
	ReasoningEfforts        *[]effort `json:"reasoning_efforts,omitzero" check:"nullabsent"`
}

//demi:wire open
type effort struct {
	ID      *string `json:"id,omitzero" check:"nullabsent,chars=1.."`
	Value   *string `json:"value,omitzero" check:"nullabsent,chars=1.."`
	Default *bool   `json:"default,omitzero" check:"nullabsent"`
}

func (p *Provider) ListModels(ctx context.Context) (core.ProviderModelList, error) {
	secret, failure := p.credentials(ctx, p.http, nil)
	if failure != nil {
		return core.ProviderModelList{}, failure.CatalogError()
	}
	headers := identityHeaders(secret)
	headers.Set("Accept", "application/json")
	response, err := send(ctx, p.http, http.MethodGet, p.modelsURL, headers, nil)
	if err != nil {
		return core.ProviderModelList{}, &provider.CatalogError{Kind: provider.CatalogUnavailable, Message: provider.TransportFailure("Grok Build models", err).Message}
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		response.Body.Close()
		return core.ProviderModelList{}, &provider.CatalogError{Kind: provider.CatalogUnavailable, Message: fmt.Sprintf("Grok Build models request failed with HTTP %d", response.StatusCode)}
	}
	raw, err := readResponse(response)
	if err != nil {
		return core.ProviderModelList{}, &provider.CatalogError{Kind: provider.CatalogUnavailable, Message: "Grok Build models response could not be read"}
	}
	var listed []model
	// Both catalog envelopes are vendor contracts; each model uses its generated decoder.
	if len(raw) > 0 {
		var envelope modelsEnvelope
		if err = json.Unmarshal(raw, &listed); err != nil {
			envelope, err = decode[modelsEnvelope](raw)
			if envelope.Data != nil {
				listed = *envelope.Data
			}
		}
	}
	if err != nil {
		return core.ProviderModelList{}, &provider.CatalogError{Kind: provider.CatalogInvalid, Message: fmt.Sprintf("Grok Build models answer cannot be read: %v", err)}
	}
	result := core.ProviderModelList{Models: []core.ProviderModel{}, Warnings: []string{}, SourceFetchedAt: p.clock.Now()}
	for _, m := range listed {
		id := m.ID
		if id == nil {
			id = m.Model
		}
		if id == nil {
			continue
		}
		name := id
		if m.Name != nil {
			name = m.Name
		}
		efforts := []string{}
		var flagged *string
		if m.ReasoningEfforts != nil {
			for _, e := range *m.ReasoningEfforts {
				name := e.ID
				if name == nil {
					name = e.Value
				}
				if name == nil {
					continue
				}
				if flagged == nil && e.Default != nil && *e.Default {
					flagged = name
				}
				efforts = append(efforts, *name)
			}
		}
		if flagged == nil {
			flagged = m.ReasoningEffort
		}
		if flagged == nil && len(efforts) > 0 {
			flagged = &efforts[0]
		}
		mapped := core.ProviderModel{ID: *id, DisplayName: *name, Description: m.Description, ContextWindow: m.ContextWindow, SupportsTools: new(true), SupportsAttachments: new(true), DefaultThinkingEffort: flagged, ServiceTiers: []core.ServiceTier{}}
		if len(efforts) > 0 {
			mapped.SupportedThinkingEfforts = &efforts
		}
		if len(efforts) > 0 || m.SupportsReasoningEffort != nil && *m.SupportsReasoningEffort {
			mapped.SupportsReasoning = new(true)
		}
		result.Models = append(result.Models, mapped)
	}
	if len(result.Models) == 0 {
		return core.ProviderModelList{}, &provider.CatalogError{Kind: provider.CatalogInvalid, Message: "Grok Build models answer lists no model"}
	}
	result.DefaultModelID = &result.Models[0].ID
	return result, nil
}
