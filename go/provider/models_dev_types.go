package provider

import (
	"encoding/json/jsontext"
	"encoding/json/v2"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/internal/wire"
)

//demi:wire open
type ModelsDevVendor struct {
	ID      string          `json:"id"`
	Name    string          `json:"name"`
	NPM     *string         `json:"npm,omitzero" check:"nullabsent"`
	API     *string         `json:"api,omitzero" check:"nullabsent"`
	Doc     *string         `json:"doc,omitzero" check:"nullabsent"`
	Entries modelsDevModels `json:"models"`
}

//demi:wire open
type modelsDevModel struct {
	Name             *string                     `json:"name,omitzero" check:"nullabsent"`
	Description      *string                     `json:"description,omitzero" check:"nullabsent"`
	Attachment       *bool                       `json:"attachment,omitzero" check:"nullabsent"`
	Reasoning        *bool                       `json:"reasoning,omitzero" check:"nullabsent"`
	ReasoningOptions *[]modelsDevReasoningOption `json:"reasoning_options,omitzero" check:"nullabsent"`
	ToolCall         *bool                       `json:"tool_call,omitzero" check:"nullabsent"`
	Limit            *modelsDevLimit             `json:"limit,omitzero" check:"nullabsent"`
	Cost             *modelsDevCost              `json:"cost,omitzero" check:"nullabsent"`
}

//demi:wire open
type modelsDevReasoningOption struct {
	Kind   string            `json:"type"`
	Values *[]nullableString `json:"values,omitzero" check:"nullabsent"`
}

//demi:wire open
type modelsDevLimit struct {
	Context *float64 `json:"context,omitzero" check:"nullabsent"`
	Output  *float64 `json:"output,omitzero" check:"nullabsent"`
}

//demi:wire open
type modelsDevCost struct {
	Input      *float64 `json:"input,omitzero" check:"nullabsent"`
	Output     *float64 `json:"output,omitzero" check:"nullabsent"`
	CacheRead  *float64 `json:"cache_read,omitzero" check:"nullabsent"`
	CacheWrite *float64 `json:"cache_write,omitzero" check:"nullabsent"`
}

//demi:opaque
type nullableString struct{ Value *string }

func (s *nullableString) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	s.Value = nil
	if wire.IsNull(dec) {
		_, err := dec.ReadToken()
		return err
	}
	value, err := wire.ReadString(dec)
	if err != nil {
		return err
	}
	s.Value = &value
	return nil
}
func (s nullableString) MarshalJSONTo(enc *jsontext.Encoder) error {
	return json.MarshalEncode(enc, s.Value)
}

// modelsDevModels preserves the vendor's document order, which catalogs expose.
//
//demi:opaque
type modelsDevModels struct {
	ids    []string
	models []modelsDevModel
}

func (m *modelsDevModels) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	*m = modelsDevModels{}
	if err := wire.BeginObject(dec); err != nil {
		return err
	}
	for {
		name, more, err := wire.NextMember(dec)
		if err != nil {
			return err
		}
		if !more {
			return nil
		}
		var model modelsDevModel
		if err := model.UnmarshalJSONFrom(dec); err != nil {
			return wire.In(name, err)
		}
		m.ids = append(m.ids, name)
		m.models = append(m.models, model)
	}
}
func (v ModelsDevVendor) Models() []core.ProviderModel {
	models := make([]core.ProviderModel, 0, len(v.Entries.ids))
	for i, id := range v.Entries.ids {
		models = append(models, v.Entries.models[i].catalog(id))
	}
	return models
}
func (m modelsDevModel) catalog(id string) core.ProviderModel {
	name := id
	if m.Name != nil {
		name = *m.Name
	}
	model := core.ProviderModel{ID: id, DisplayName: name, Description: m.Description, SupportsTools: m.ToolCall, SupportsAttachments: m.Attachment, SupportsReasoning: m.Reasoning, ServiceTiers: []core.ServiceTier{}}
	if m.Limit != nil {
		model.ContextWindow = modelTokens(m.Limit.Context)
		model.OutputLimit = modelTokens(m.Limit.Output)
	}
	if m.Cost != nil {
		model.Cost = &core.ModelCost{Input: m.Cost.Input, Output: m.Cost.Output, CacheRead: m.Cost.CacheRead, CacheWrite: m.Cost.CacheWrite}
	}
	if m.ReasoningOptions != nil {
		for _, option := range *m.ReasoningOptions {
			if option.Kind != "effort" {
				continue
			}
			if option.Values != nil {
				values := []string{}
				for _, value := range *option.Values {
					if value.Value != nil && *value.Value != "" {
						values = append(values, *value.Value)
					}
				}
				model.SupportedThinkingEfforts = &values
			}
			break
		}
	}
	return model
}
func modelTokens(value *float64) *uint32 {
	if value == nil || *value < 1 || *value > 4294967295 {
		return nil
	}
	integer := uint32(*value)
	if float64(integer) != *value {
		return nil
	}
	return &integer
}
