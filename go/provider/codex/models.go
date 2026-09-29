package codex

import (
	"context"
	"errors"
	"fmt"
	"io"
	"slices"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

//demi:wire open
type modelsAnswer struct {
	Models []model `json:"models"`
}

//demi:wire open
type model struct {
	Slug               string           `json:"slug" check:"chars=1.."`
	DisplayName        string           `json:"display_name" check:"chars=1.."`
	Description        *string          `json:"description,omitzero" check:"nullabsent"`
	Visibility         string           `json:"visibility" check:"oneof=list|hide|none"`
	Priority           int64            `json:"priority"`
	ContextWindow      *uint32          `json:"context_window,omitzero" check:"nullabsent,range=1.."`
	InputModalities    *[]string        `json:"input_modalities,omitzero" check:"nullabsent"`
	Reasoning          []reasoningLevel `json:"supported_reasoning_levels"`
	DefaultReasoning   *string          `json:"default_reasoning_level,omitzero" check:"nullabsent,chars=1.."`
	ServiceTiers       *[]tier          `json:"service_tiers,omitzero" check:"nullabsent"`
	DefaultServiceTier *string          `json:"default_service_tier,omitzero" check:"nullabsent,chars=1.."`
	ToolMode           *string          `json:"tool_mode,omitzero" check:"nullabsent"`
	ExperimentalTools  *[]string        `json:"experimental_supported_tools,omitzero" check:"nullabsent"`
	// Three states retain explicit null for capability detection.
	ApplyPatch **string `json:"apply_patch_tool_type,omitzero" check:"nullable"`
	WebSearch  **string `json:"web_search_tool_type,omitzero" check:"nullable"`
}

//demi:wire open
type reasoningLevel struct {
	Effort string `json:"effort" check:"chars=1.."`
}

//demi:wire open
type tier struct {
	ID          string  `json:"id" check:"chars=1.."`
	Name        string  `json:"name" check:"chars=1.."`
	Description *string `json:"description,omitzero" check:"nullabsent"`
}

func (p *Provider) ListModels(ctx context.Context) (core.ProviderModelList, error) {
	response, err := p.get(ctx, p.modelsURL, true)
	if err != nil {
		var failure *provider.AuthFailure
		if errors.As(err, &failure) {
			return core.ProviderModelList{}, failure.CatalogError()
		}
		return core.ProviderModelList{}, &provider.CatalogError{Kind: provider.CatalogUnavailable, Message: err.Error()}
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return core.ProviderModelList{}, &provider.CatalogError{Kind: provider.CatalogUnavailable, Message: fmt.Sprintf("Codex models request failed with HTTP %d", response.StatusCode)}
	}
	raw, err := io.ReadAll(response.Body)
	if err != nil {
		return core.ProviderModelList{}, &provider.CatalogError{Kind: provider.CatalogUnavailable, Message: provider.TransportFailure("Codex models", err).Message}
	}
	answer, err := decode[modelsAnswer](raw)
	if err != nil {
		return core.ProviderModelList{}, &provider.CatalogError{Kind: provider.CatalogInvalid, Message: fmt.Sprintf("Codex models answer cannot be read: %v", err)}
	}
	listed := slices.DeleteFunc(answer.Models, func(model model) bool { return model.Visibility != "list" })
	slices.SortStableFunc(listed, func(a, b model) int {
		if a.Priority < b.Priority {
			return -1
		}
		if a.Priority > b.Priority {
			return 1
		}
		return 0
	})
	models := make([]core.ProviderModel, 0, len(listed))
	for _, model := range listed {
		models = append(models, model.catalog())
	}
	result := core.ProviderModelList{Models: models, Warnings: []string{}, SourceFetchedAt: p.clock.Now()}
	if len(models) > 0 {
		result.DefaultModelID = &models[0].ID
	}
	return result, nil
}
func (m model) catalog() core.ProviderModel {
	efforts := make([]string, 0, len(m.Reasoning))
	for _, level := range m.Reasoning {
		efforts = append(efforts, level.Effort)
	}
	result := core.ProviderModel{ID: m.Slug, DisplayName: m.DisplayName, Description: m.Description, ContextWindow: m.ContextWindow, SupportsReasoning: new(len(efforts) > 0), SupportedThinkingEfforts: &efforts, DefaultThinkingEffort: m.DefaultReasoning, CanDisableThinking: new(false), DefaultServiceTierID: m.DefaultServiceTier, ServiceTiers: []core.ServiceTier{}}
	if m.ToolMode != nil && *m.ToolMode != "" {
		result.SupportsTools = new(true)
	} else if m.ExperimentalTools != nil {
		result.SupportsTools = new(len(*m.ExperimentalTools) > 0)
	} else if m.ApplyPatch != nil || m.WebSearch != nil {
		result.SupportsTools = new(true)
	}
	if m.InputModalities != nil {
		result.SupportsAttachments = new(slices.Contains(*m.InputModalities, "image"))
	}
	if m.ServiceTiers != nil {
		for _, tier := range *m.ServiceTiers {
			description := tier.Description
			if description != nil && *description == "" {
				description = nil
			}
			result.ServiceTiers = append(result.ServiceTiers, core.ServiceTier{ID: tier.ID, Label: tier.Name, Description: description, Fast: tier.ID == "priority"})
		}
	}
	return result
}
