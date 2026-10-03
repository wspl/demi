package core_test

import (
	"errors"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/core"
)

// Catalog conversion is pure dense logic; the complete table costs under one second.
func TestCatalogSelection(t *testing.T) {
	catalog := core.ProviderModel{
		ID:                  "m-1",
		DisplayName:         "Model One",
		ContextWindow:       new(uint32(200000)),
		OutputLimit:         new(uint32(8000)),
		SupportsAttachments: new(true),
	}
	thinking := &core.EffortConfig{Effort: "high"}
	got := catalog.Selection("p", thinking, new("priority"))
	want := core.ModelSelection{
		ProviderID: "p",
		Model: core.Model{
			ID:                 "m-1",
			Name:               "Model One",
			ContextWindow:      200000,
			OutputLimit:        new(uint32(8000)),
			Thinking:           []core.ThinkingCapability{},
			AcceptedExtensions: &core.AttachmentFileExtensions,
		},
		Thinking:      thinking,
		ServiceTierID: new("priority"),
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("%+v, want %+v", got, want)
	}
	plain := (core.ProviderModel{}).Selection("p", nil, nil)
	if plain.Model.ContextWindow != 0 || plain.Thinking != nil || plain.ServiceTierID != nil {
		t.Fatalf("%+v", plain)
	}
}

func TestCatalogAcceptedTypes(t *testing.T) {
	both := append(append([]core.FileExtension{}, core.AttachmentFileExtensions...), core.VideoFileExtensions...)
	for _, scenario := range []struct {
		attachments, video *bool
		exact, want        *[]core.FileExtension
	}{
		{nil, nil, nil, nil},
		{nil, new(false), nil, nil},
		{new(false), nil, nil, new([]core.FileExtension{})},
		{new(true), nil, nil, &core.AttachmentFileExtensions},
		{new(true), new(false), nil, &core.AttachmentFileExtensions},
		{new(true), new(true), nil, &both},
		{new(false), new(true), nil, &core.VideoFileExtensions},
		{nil, new(true), nil, &core.VideoFileExtensions},
		{new(true), nil, new([]core.FileExtension{"png"}), new([]core.FileExtension{"png"})},
		{new(true), new(true), new([]core.FileExtension{}), new([]core.FileExtension{})},
	} {
		model := core.ProviderModel{
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
		want      []core.ThinkingCapability
	}{
		{new(false), new([]string{"low"}), nil, []core.ThinkingCapability{&core.DisabledCapability{}}},
		{nil, nil, nil, []core.ThinkingCapability{}},
		{new(true), new([]string{}), nil, []core.ThinkingCapability{}},
		{
			new(true),
			new([]string{
				"low",
				"high",
			}),
			new("high"),
			[]core.ThinkingCapability{
				&core.EffortCapability{
					Efforts: []string{
						"low",
						"high",
					},
					DefaultEffort: new("high"),
					Summaries: []core.ThinkingSummary{
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
		model := core.ProviderModel{
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
	model := core.ProviderModel{
		SupportedThinkingEfforts: new([]string{"low", "high"}),
		CanDisableThinking:       new(false),
		ServiceTiers:             []core.ServiceTier{{ID: "priority"}},
	}
	for _, scenario := range []struct {
		effort     *string
		fallback   *string
		canDisable bool
		want       core.ThinkingConfig
		refused    bool
	}{
		{nil, new("high"), false, &core.EffortConfig{Effort: "high"}, false},
		{nil, new("unlisted"), false, &core.EffortConfig{Effort: "low"}, false},
		{nil, nil, true, nil, false},
		{new("disabled"), nil, true, &core.DisabledConfig{}, false},
		{new("disabled"), nil, false, nil, true},
		{new("ultra"), nil, true, nil, true},
		{new("high"), nil, false, &core.EffortConfig{Effort: "high"}, false},
	} {
		model.CanDisableThinking = &scenario.canDisable
		model.DefaultThinkingEffort = scenario.fallback
		got, err := model.ThinkingFor(scenario.effort)
		if (err != nil) != scenario.refused || !reflect.DeepEqual(got, scenario.want) {
			t.Errorf("%+v: %v, %v", scenario, got, err)
		}
		if scenario.refused {
			var unavailable *core.UnavailableSetting
			if !errors.As(err, &unavailable) {
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
		config core.ThinkingConfig
		want   *string
	}{
		{nil, nil},
		{&core.AdaptiveConfig{Effort: "high"}, new("high")},
		{&core.EffortConfig{Effort: "low"}, new("low")},
		{&core.DisabledConfig{}, new(core.ThinkingOff)},
		{&core.BudgetConfig{BudgetTokens: 1000}, nil},
	} {
		if got, ok := (core.ModelSelection{Thinking: scenario.config}).ThinkingEffort(); ok != (scenario.want != nil) ||
			(ok && got != *scenario.want) {
			t.Errorf("effort: %q, present: %v", got, ok)
		}
	}
}

// Results belong to the caller: changing settings must not edit a catalog or
// another selection. These pure mutation scenarios cost under one second.
func TestCatalogResultsDoNotAlias(t *testing.T) {
	t.Run("selection limits and tier", func(t *testing.T) {
		catalog := core.ProviderModel{OutputLimit: new(uint32(8000))}
		tier := "priority"
		result := catalog.Selection("p", nil, &tier)
		*result.Model.OutputLimit = 1
		*result.ServiceTierID = "changed"
		if *catalog.OutputLimit != 8000 || tier != "priority" {
			t.Fatal("selection changed the source limit or tier")
		}
	})
	t.Run("selection thinking", func(t *testing.T) {
		for _, config := range []core.ThinkingConfig{
			&core.AdaptiveConfig{Effort: "high"},
			&core.BudgetConfig{BudgetTokens: 1000},
			&core.EffortConfig{Effort: "high", Summary: new(core.ThinkingSummaryAuto)},
			&core.DisabledConfig{},
		} {
			result := (core.ProviderModel{}).Selection("p", config, nil)
			if !reflect.DeepEqual(result.Thinking, config) {
				t.Fatalf("selection changed thinking: %#v", config)
			}
			switch changed := result.Thinking.(type) {
			case *core.AdaptiveConfig:
				changed.Effort = "low"
				if config.(*core.AdaptiveConfig).Effort != "high" {
					t.Fatal("adaptive setting was shared")
				}
			case *core.BudgetConfig:
				changed.BudgetTokens = 1
				if config.(*core.BudgetConfig).BudgetTokens != 1000 {
					t.Fatal("budget setting was shared")
				}
			case *core.EffortConfig:
				changed.Effort = "low"
				*changed.Summary = core.ThinkingSummaryOff
				source := config.(*core.EffortConfig)
				if source.Effort != "high" || *source.Summary != core.ThinkingSummaryAuto {
					t.Fatal("effort setting or summary was shared")
				}
			case *core.DisabledConfig:
				// This variant has no mutable fields.
			}
		}
	})
	t.Run("capabilities", func(t *testing.T) {
		catalog := core.ProviderModel{
			SupportedThinkingEfforts: new([]string{"high"}),
			DefaultThinkingEffort:    new("high"),
		}
		result := catalog.ThinkingCapabilities()[0].(*core.EffortCapability)
		*result.DefaultEffort = "low"
		result.Efforts[0] = "low"
		if *catalog.DefaultThinkingEffort != "high" || (*catalog.SupportedThinkingEfforts)[0] != "high" {
			t.Fatal("capabilities changed the catalog")
		}
	})
	t.Run("unnamed effort", func(t *testing.T) {
		for _, fallback := range []*string{nil, new("high")} {
			catalog := core.ProviderModel{
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
		catalog := core.ProviderModel{ServiceTiers: []core.ServiceTier{{ID: "priority"}}}
		result, err := catalog.TierFor(&catalog.ServiceTiers[0].ID)
		if err != nil {
			t.Fatal(err)
		}
		*result = "changed"
		if catalog.ServiceTiers[0].ID != "priority" {
			t.Fatal("tier changed the source")
		}
	})
	t.Run("displayed thinking effort", func(t *testing.T) {
		for _, config := range []core.ThinkingConfig{
			&core.AdaptiveConfig{
				Effort: "high",
			},
			&core.EffortConfig{
				Effort: "high",
			},
		} {
			selection := core.ModelSelection{Thinking: config}
			result, ok := selection.ThinkingEffort()
			if !ok || result != "high" {
				t.Fatal("missing thinking effort")
			}
			// A caller can mutate its returned value without changing settings.
			local := &result
			*local = "low"
			if source, _ := selection.ThinkingEffort(); source != "high" {
				t.Fatal("displayed effort changed the selection")
			}
		}
	})
}
