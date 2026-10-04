package types_test

import (
	"fmt"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/types"
)

// Catalog conversion is pure dense logic; the complete table costs under one second.
func TestCatalogSelection(t *testing.T) {
	catalog := types.ProviderModel{
		ID:                  "m-1",
		DisplayName:         "Model One",
		ContextWindow:       new(uint32(200000)),
		OutputLimit:         new(uint32(8000)),
		SupportsAttachments: new(true),
	}
	thinking := &types.EffortConfig{Effort: "high"}
	got := catalog.Selection("p", thinking, new("priority"))
	want := types.ModelSelection{
		ProviderID: "p",
		Model: types.Model{
			ID:                 "m-1",
			Name:               "Model One",
			ContextWindow:      200000,
			OutputLimit:        new(uint32(8000)),
			Thinking:           []types.ThinkingCapability{},
			AcceptedExtensions: &types.AttachmentFileExtensions,
		},
		Thinking:      thinking,
		ServiceTierID: new("priority"),
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("%+v, want %+v", got, want)
	}
	plain := (types.ProviderModel{}).Selection("p", nil, nil)
	if plain.Model.ContextWindow != 0 || plain.Thinking != nil || plain.ServiceTierID != nil {
		t.Fatalf("%+v", plain)
	}
}

func TestCatalogAcceptedTypes(t *testing.T) {
	both := append(append([]types.FileExtension{}, types.AttachmentFileExtensions...), types.VideoFileExtensions...)
	for _, scenario := range []struct {
		attachments, video *bool
		exact, want        *[]types.FileExtension
	}{
		{nil, nil, nil, nil},
		{nil, new(false), nil, nil},
		{new(false), nil, nil, new([]types.FileExtension{})},
		{new(true), nil, nil, &types.AttachmentFileExtensions},
		{new(true), new(false), nil, &types.AttachmentFileExtensions},
		{new(true), new(true), nil, &both},
		{new(false), new(true), nil, &types.VideoFileExtensions},
		{nil, new(true), nil, &types.VideoFileExtensions},
		{new(true), nil, new([]types.FileExtension{"png"}), new([]types.FileExtension{"png"})},
		{new(true), new(true), new([]types.FileExtension{}), new([]types.FileExtension{})},
	} {
		model := types.ProviderModel{
			SupportsAttachments: scenario.attachments,
			SupportsVideo:       scenario.video,
			AcceptedExtensions:  scenario.exact,
		}
		if got := model.Selection("p", nil, nil).Model.AcceptedExtensions; !reflect.DeepEqual(got, scenario.want) {
			t.Errorf("%+v: %v, want %v", scenario, got, scenario.want)
		}
	}
}

func TestCatalogThinking(t *testing.T) {
	for _, scenario := range []struct {
		reasoning *bool
		efforts   *[]string
		fallback  *string
		want      []types.ThinkingCapability
	}{
		{new(false), new([]string{"low"}), nil, []types.ThinkingCapability{&types.DisabledCapability{}}},
		{nil, nil, nil, []types.ThinkingCapability{}},
		{new(true), new([]string{}), nil, []types.ThinkingCapability{}},
		{
			new(true),
			new([]string{
				"low",
				"high",
			}),
			new("high"),
			[]types.ThinkingCapability{
				&types.EffortCapability{
					Efforts: []string{
						"low",
						"high",
					},
					DefaultEffort: new("high"),
					Summaries: []types.ThinkingSummary{
						"auto",
						"concise",
						"detailed",
						"off",
						"on",
					},
				},
			},
		},
	} {
		model := types.ProviderModel{
			SupportsReasoning:        scenario.reasoning,
			SupportedThinkingEfforts: scenario.efforts,
			DefaultThinkingEffort:    scenario.fallback,
		}
		if got := model.Selection("p", nil, nil).Model.Thinking; !reflect.DeepEqual(got, scenario.want) {
			t.Errorf("%#v, want %#v", got, scenario.want)
		}
	}
}

func TestModelSettings(t *testing.T) {
	model := types.ProviderModel{
		SupportedThinkingEfforts: new([]string{"low", "high"}),
		CanDisableThinking:       new(false),
		ServiceTiers:             []types.ServiceTier{{ID: "priority"}},
	}
	for _, scenario := range []struct {
		effort     *string
		fallback   *string
		canDisable bool
		want       types.ThinkingConfig
		refused    bool
	}{
		{nil, new("high"), false, &types.EffortConfig{Effort: "high"}, false},
		{nil, new("unlisted"), false, &types.EffortConfig{Effort: "low"}, false},
		{nil, nil, true, nil, false},
		{new("disabled"), nil, true, &types.DisabledConfig{}, false},
		{new("disabled"), nil, false, nil, true},
		{new("ultra"), nil, true, nil, true},
		{new("high"), nil, false, &types.EffortConfig{Effort: "high"}, false},
	} {
		model.CanDisableThinking = &scenario.canDisable
		model.DefaultThinkingEffort = scenario.fallback
		got, err := model.ThinkingFor(scenario.effort)
		if (err != nil) != scenario.refused || !reflect.DeepEqual(got, scenario.want) {
			t.Errorf("%+v: %v, %v", scenario, got, err)
		}
		if scenario.refused && err != nil {
			want := fmt.Sprintf("The model does not offer the thinking effort %q", *scenario.effort)
			if err.Error() != want {
				t.Errorf("wrong error: %v", err)
			}
		}
	}
	for _, tier := range []*string{nil, new("priority"), new("missing")} {
		got, err := model.TierFor(tier)
		if tier != nil && *tier == "missing" {
			if err == nil {
				t.Fatal("unlisted tier accepted")
			}
		} else if err != nil || !reflect.DeepEqual(got, tier) {
			t.Fatalf("%v: %v", got, err)
		}
	}
	for _, scenario := range []struct {
		config types.ThinkingConfig
		want   *string
	}{
		{nil, nil},
		{&types.AdaptiveConfig{Effort: "high"}, new("high")},
		{&types.EffortConfig{Effort: "low"}, new("low")},
		{&types.DisabledConfig{}, new(types.ThinkingOff)},
		{&types.BudgetConfig{BudgetTokens: 1000}, nil},
	} {
		if got, ok := (types.ModelSelection{Thinking: scenario.config}).ThinkingEffort(); ok != (scenario.want != nil) ||
			(ok && got != *scenario.want) {
			t.Errorf("effort: %q, present: %v", got, ok)
		}
	}
}

// Results belong to the caller: changing settings must not edit a catalog or
// another selection. These pure mutation scenarios cost under one second.
func TestCatalogResultsDoNotAlias(t *testing.T) {
	t.Run("selection limits and tier", func(t *testing.T) {
		catalog := types.ProviderModel{OutputLimit: new(uint32(8000))}
		tier := "priority"
		result := catalog.Selection("p", nil, &tier)
		*result.Model.OutputLimit = 1
		*result.ServiceTierID = "changed"
		if *catalog.OutputLimit != 8000 || tier != "priority" {
			t.Fatal("selection changed the source limit or tier")
		}
	})
	t.Run("selection thinking", func(t *testing.T) {
		for _, config := range []types.ThinkingConfig{
			&types.AdaptiveConfig{Effort: "high"},
			&types.BudgetConfig{BudgetTokens: 1000},
			&types.EffortConfig{Effort: "high", Summary: new(types.ThinkingSummaryAuto)},
			&types.DisabledConfig{},
		} {
			result := (types.ProviderModel{}).Selection("p", config, nil)
			if !reflect.DeepEqual(result.Thinking, config) {
				t.Fatalf("selection changed thinking: %#v", config)
			}
			switch changed := result.Thinking.(type) {
			case *types.AdaptiveConfig:
				changed.Effort = "low"
				if config.(*types.AdaptiveConfig).Effort != "high" {
					t.Fatal("adaptive setting was shared")
				}
			case *types.BudgetConfig:
				changed.BudgetTokens = 1
				if config.(*types.BudgetConfig).BudgetTokens != 1000 {
					t.Fatal("budget setting was shared")
				}
			case *types.EffortConfig:
				changed.Effort = "low"
				*changed.Summary = types.ThinkingSummaryOff
				source := config.(*types.EffortConfig)
				if source.Effort != "high" || *source.Summary != types.ThinkingSummaryAuto {
					t.Fatal("effort setting or summary was shared")
				}
			case *types.DisabledConfig:
				// This variant has no mutable fields.
			}
		}
	})
	t.Run("capabilities", func(t *testing.T) {
		catalog := types.ProviderModel{
			SupportedThinkingEfforts: new([]string{"high"}),
			DefaultThinkingEffort:    new("high"),
		}
		result := catalog.ThinkingCapabilities()[0].(*types.EffortCapability)
		*result.DefaultEffort = "low"
		result.Efforts[0] = "low"
		if *catalog.DefaultThinkingEffort != "high" || (*catalog.SupportedThinkingEfforts)[0] != "high" {
			t.Fatal("capabilities changed the catalog")
		}
	})
	t.Run("unnamed effort", func(t *testing.T) {
		for _, fallback := range []*string{nil, new("high")} {
			catalog := types.ProviderModel{
				CanDisableThinking:       new(false),
				SupportedThinkingEfforts: new([]string{"high"}),
				DefaultThinkingEffort:    fallback,
			}
			result := catalog.UnnamedEffort()
			*result = "low"
			if (*catalog.SupportedThinkingEfforts)[0] != "high" || (fallback != nil && *fallback != "high") {
				t.Fatal("default effort changed the catalog")
			}
		}
	})
	t.Run("tier", func(t *testing.T) {
		catalog := types.ProviderModel{ServiceTiers: []types.ServiceTier{{ID: "priority"}}}
		result, err := catalog.TierFor(&catalog.ServiceTiers[0].ID)
		if err != nil {
			t.Fatal(err)
		}
		*result = "changed"
		if catalog.ServiceTiers[0].ID != "priority" {
			t.Fatal("tier changed the source")
		}
	})
}
