package codex

import (
	"cmp"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"slices"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

type catalogModel struct {
	Slug         provider.NonEmpty  `json:"slug"`
	Name         provider.NonEmpty  `json:"display_name"`
	Description  *string            `json:"description"`
	Visibility   string             `json:"visibility"`
	Priority     int64              `json:"priority"`
	Context      *uint32            `json:"context_window"`
	Modalities   *[]string          `json:"input_modalities"`
	Levels       []reasoningLevel   `json:"supported_reasoning_levels"`
	DefaultLevel *provider.NonEmpty `json:"default_reasoning_level"`
	Tiers        *[]catalogTier     `json:"service_tiers"`
	DefaultTier  *provider.NonEmpty `json:"default_service_tier"`
	ToolMode     *string            `json:"tool_mode"`
	Tools        *[]string          `json:"experimental_supported_tools"`
	Patch        json.RawMessage    `json:"apply_patch_tool_type"        wire:"optional"`
	Search       json.RawMessage    `json:"web_search_tool_type"         wire:"optional"`
}
type reasoningLevel struct {
	Effort provider.NonEmpty `json:"effort"`
}
type catalogTier struct {
	ID          provider.NonEmpty `json:"id"`
	Name        provider.NonEmpty `json:"name"`
	Description *string           `json:"description"`
}

// ListModels reads the account's current model picker, refreshing once on a 401.
//
//nolint:staticcheck // ST1005: user-facing error text starts with a capital letter.
func (p *Provider) ListModels(ctx context.Context) (types.ProviderModelList, error) {
	var refused *provider.Secret
	for {
		s, err := p.credentials(ctx, p.http, refused)
		if err != nil {
			return types.ProviderModelList{}, err
		}
		request, err := http.NewRequestWithContext(ctx, http.MethodGet, p.modelsURL, nil)
		if err != nil {
			return types.ProviderModelList{}, err
		}
		request.Header = accountHeaders(s)
		request.Header.Set("Accept", "application/json")
		response, err := p.http.Do(request)
		if err != nil {
			return types.ProviderModelList{}, fmt.Errorf("Codex models request failed: %v", provider.WithoutURL(err))
		}
		if response.StatusCode == 401 && refused == nil {
			_ = response.Body.Close() // A refused catalog contributes only its status.
			refused = &s.AccessToken
			continue
		}
		if response.StatusCode < 200 || response.StatusCode >= 300 {
			_ = response.Body.Close() // A refused catalog contributes only its status.
			return types.ProviderModelList{}, fmt.Errorf(
				"Codex models request failed with HTTP %d",
				response.StatusCode,
			)
		}
		data, readErr := io.ReadAll(response.Body)
		_ = response.Body.Close() // Read reports transfer failures; close releases the body.
		if readErr != nil {
			return types.ProviderModelList{}, fmt.Errorf("Codex models request failed: %v", readErr)
		}
		result, err := p.catalog(string(data))
		if err != nil {
			return types.ProviderModelList{}, fmt.Errorf("Codex models answer cannot be read: %v", err)
		}
		return result, nil
	}
}

func (p *Provider) catalog(text string) (types.ProviderModelList, error) {
	answer, err := provider.DecodeUntagged[struct {
		Models []catalogModel `json:"models"`
	}](text)
	if err != nil {
		return types.ProviderModelList{}, err
	}
	for _, m := range answer.Models {
		if m.Visibility != "list" && m.Visibility != "hide" && m.Visibility != "none" {
			return types.ProviderModelList{}, fmt.Errorf("unknown visibility %q", m.Visibility)
		}
		if m.Context != nil && *m.Context == 0 {
			return types.ProviderModelList{}, fmt.Errorf("context_window must be nonzero")
		}
		for _, raw := range []json.RawMessage{m.Patch, m.Search} {
			if len(raw) != 0 {
				if _, err := provider.DecodeUntagged[*string](string(raw)); err != nil {
					return types.ProviderModelList{}, err
				}
			}
		}
	}
	slices.SortStableFunc(answer.Models, func(a, b catalogModel) int {
		return cmp.Compare(a.Priority, b.Priority)
	})
	result := types.ProviderModelList{
		Models:          []types.ProviderModel{},
		Warnings:        []string{},
		SourceFetchedAt: p.clock.Now(),
	}
	for _, m := range answer.Models {
		if m.Visibility != "list" {
			continue
		}
		model := m.model()
		result.Models = append(result.Models, model)
	}
	if len(result.Models) > 0 {
		result.DefaultModelID = &result.Models[0].ID
	}
	return result, nil
}

func (m catalogModel) model() types.ProviderModel {
	efforts := make([]string, 0, len(m.Levels))
	for _, level := range m.Levels {
		efforts = append(efforts, string(level.Effort))
	}
	reasoning, disable := len(efforts) > 0, false
	model := types.ProviderModel{
		ID:                       string(m.Slug),
		DisplayName:              string(m.Name),
		Description:              m.Description,
		ContextWindow:            m.Context,
		SupportsReasoning:        &reasoning,
		SupportedThinkingEfforts: &efforts,
		CanDisableThinking:       &disable,
		ServiceTiers:             []types.ServiceTier{},
	}
	if m.Modalities != nil {
		v := slices.Contains(*m.Modalities, "image")
		model.SupportsAttachments = &v
	}
	switch {
	case m.ToolMode != nil && *m.ToolMode != "":
		v := true
		model.SupportsTools = &v
	case m.Tools != nil:
		v := len(*m.Tools) > 0
		model.SupportsTools = &v
	case len(m.Patch) != 0 || len(m.Search) != 0:
		v := true
		model.SupportsTools = &v
	}
	if m.DefaultLevel != nil {
		value := string(*m.DefaultLevel)
		model.DefaultThinkingEffort = &value
	}
	if m.DefaultTier != nil {
		value := string(*m.DefaultTier)
		model.DefaultServiceTierID = &value
	}
	if m.Tiers != nil {
		for _, tier := range *m.Tiers {
			description := tier.Description
			if description != nil && *description == "" {
				description = nil
			}
			model.ServiceTiers = append(
				model.ServiceTiers,
				types.ServiceTier{
					ID:          string(tier.ID),
					Label:       string(tier.Name),
					Description: description,
					Fast:        tier.ID == "priority",
				},
			)
		}
	}
	return model
}
