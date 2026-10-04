package grokbuild

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

type catalogModel struct {
	ID             *provider.NonEmpty `json:"id"`
	Model          *provider.NonEmpty `json:"model"`
	Name           *provider.NonEmpty `json:"name"`
	Description    *string            `json:"description"`
	ContextWindow  *uint32            `json:"context_window"`
	SupportsEffort *bool              `json:"supports_reasoning_effort"`
	Effort         *provider.NonEmpty `json:"reasoning_effort"`
	Efforts        *[]catalogEffort   `json:"reasoning_efforts"`
}
type catalogEffort struct {
	ID      *provider.NonEmpty `json:"id"`
	Value   *provider.NonEmpty `json:"value"`
	Default *bool              `json:"default"`
}

// ListModels reads a fresh catalog from the chat proxy.
//
//nolint:staticcheck // ST1005: user-facing error text starts with a capital letter.
func (p *Provider) ListModels(ctx context.Context) (types.ProviderModelList, error) {
	s, failure := p.auth.credentials(ctx, p.http, nil)
	if failure != nil {
		return types.ProviderModelList{}, failure
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, p.modelsURL.String(), nil)
	if err != nil {
		return types.ProviderModelList{}, fmt.Errorf("Grok Build models request failed: %v", provider.WithoutURL(err))
	}
	req.Header = identityHeaders(s)
	req.Header.Set("Accept", "application/json")
	response, err := p.http.Do(req)
	if err != nil {
		return types.ProviderModelList{}, fmt.Errorf("Grok Build models request failed: %v", provider.WithoutURL(err))
	}
	// The response is consumed or abandoned; close errors cannot change its result.
	defer func() {
		_ = response.Body.Close()
	}()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return types.ProviderModelList{}, fmt.Errorf(
			"Grok Build models request failed with HTTP %d",
			response.StatusCode,
		)
	}
	data, err := io.ReadAll(response.Body)
	if err != nil {
		return types.ProviderModelList{}, fmt.Errorf("Grok Build models request failed: %v", provider.WithoutURL(err))
	}
	return p.catalog(string(data))
}

//nolint:staticcheck // ST1005: user-facing error text starts with a capital letter.
func (p *Provider) catalog(text string) (types.ProviderModelList, error) {
	var listed []catalogModel
	var err error
	if strings.HasPrefix(strings.TrimSpace(text), "[") {
		listed, err = provider.DecodeUntagged[[]catalogModel](text)
	} else {
		var envelope struct {
			Data *[]catalogModel `json:"data"`
		}
		envelope, err = provider.DecodeUntagged[struct {
			Data *[]catalogModel `json:"data"`
		}](text)
		if envelope.Data != nil {
			listed = *envelope.Data
		}
	}
	invalid := func(err error) (types.ProviderModelList, error) {
		return types.ProviderModelList{}, fmt.Errorf("Grok Build models answer cannot be read: %v", err)
	}
	if err != nil {
		return invalid(err)
	}
	models := make([]types.ProviderModel, 0, len(listed))
	for _, m := range listed {
		if m.ContextWindow != nil && *m.ContextWindow == 0 {
			return invalid(fmt.Errorf("context_window: expected a nonzero count"))
		}
		id := m.ID
		if id == nil {
			id = m.Model
		}
		if id == nil {
			continue
		}
		name := string(*id)
		if m.Name != nil {
			name = string(*m.Name)
		}
		model := m.model(id, name)
		models = append(models, model)
	}
	if len(models) == 0 {
		return types.ProviderModelList{}, errors.New("Grok Build models answer lists no model")
	}
	return types.ProviderModelList{
		Models:          models,
		DefaultModelID:  &models[0].ID,
		Warnings:        []string{},
		SourceFetchedAt: p.clock.Now(),
	}, nil
}

func (m catalogModel) model(id *provider.NonEmpty, name string) types.ProviderModel {
	efforts := []string{}
	var chosen *string
	if m.Efforts != nil {
		for _, e := range *m.Efforts {
			name := e.ID
			if name == nil {
				name = e.Value
			}
			if name == nil {
				continue
			}
			value := string(*name)
			if chosen == nil && e.Default != nil && *e.Default {
				chosen = &value
			}
			efforts = append(efforts, value)
		}
	}
	if chosen == nil && m.Effort != nil {
		value := string(*m.Effort)
		chosen = &value
	}
	if chosen == nil && len(efforts) > 0 {
		chosen = &efforts[0]
	}
	yes := true
	model := types.ProviderModel{
		ID:                    string(*id),
		DisplayName:           name,
		Description:           m.Description,
		ContextWindow:         m.ContextWindow,
		SupportsTools:         &yes,
		SupportsAttachments:   &yes,
		DefaultThinkingEffort: chosen,
		ServiceTiers:          []types.ServiceTier{},
	}
	if len(efforts) > 0 || (m.SupportsEffort != nil && *m.SupportsEffort) {
		model.SupportsReasoning = &yes
	}
	if len(efforts) > 0 {
		model.SupportedThinkingEfforts = &efforts
	}
	return model
}
