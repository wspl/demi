package core_test

import (
	"errors"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/core"
)

// Catalog conversion is pure dense logic; the complete table costs under one second.
func TestCatalogSelection(t *testing.T) {
	catalog := core.ProviderModel{ID: "m-1", DisplayName: "Model One", ContextWindow: new(uint32(200000)), OutputLimit: new(uint32(8000)), SupportsAttachments: new(true)}
	thinking := &core.EffortConfig{Effort: "high"}
	got := catalog.Selection("p", thinking, new("priority"))
	want := core.ModelSelection{ProviderID: "p", Model: core.Model{ID: "m-1", Name: "Model One", ContextWindow: 200000, OutputLimit: new(uint32(8000)), Thinking: []core.ThinkingCapability{}, AcceptedExtensions: &core.AttachmentFileExtensions}, Thinking: thinking, ServiceTierID: new("priority")}
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
	for _, tc := range []struct {
		attachments, video *bool
		exact, want        *[]core.FileExtension
	}{
		{nil, nil, nil, nil}, {nil, new(false), nil, nil}, {new(false), nil, nil, new([]core.FileExtension{})},
		{new(true), nil, nil, &core.AttachmentFileExtensions}, {new(true), new(false), nil, &core.AttachmentFileExtensions},
		{new(true), new(true), nil, &both}, {new(false), new(true), nil, &core.VideoFileExtensions}, {nil, new(true), nil, &core.VideoFileExtensions},
		{new(true), nil, new([]core.FileExtension{"png"}), new([]core.FileExtension{"png"})},
		{new(true), new(true), new([]core.FileExtension{}), new([]core.FileExtension{})},
	} {
		model := core.ProviderModel{SupportsAttachments: tc.attachments, SupportsVideo: tc.video, AcceptedExtensions: tc.exact}
		if got := model.Selection("p", nil, nil).Model.AcceptedExtensions; !reflect.DeepEqual(got, tc.want) {
			t.Errorf("%+v: %v, want %v", tc, got, tc.want)
		}
	}
}

func TestCatalogThinking(t *testing.T) {
	for _, tc := range []struct {
		reasoning *bool
		efforts   *[]string
		fallback  *string
		want      []core.ThinkingCapability
	}{
		{new(false), new([]string{"low"}), nil, []core.ThinkingCapability{&core.DisabledCapability{}}},
		{nil, nil, nil, []core.ThinkingCapability{}}, {new(true), new([]string{}), nil, []core.ThinkingCapability{}},
		{new(true), new([]string{"low", "high"}), new("high"), []core.ThinkingCapability{&core.EffortCapability{Efforts: []string{"low", "high"}, DefaultEffort: new("high"), Summaries: []core.ThinkingSummary{"auto", "concise", "detailed", "off", "on"}}}},
	} {
		model := core.ProviderModel{SupportsReasoning: tc.reasoning, SupportedThinkingEfforts: tc.efforts, DefaultThinkingEffort: tc.fallback}
		if got := model.Selection("p", nil, nil).Model.Thinking; !reflect.DeepEqual(got, tc.want) {
			t.Errorf("%#v, want %#v", got, tc.want)
		}
	}
}

func TestModelSettings(t *testing.T) {
	model := core.ProviderModel{SupportedThinkingEfforts: new([]string{"low", "high"}), CanDisableThinking: new(false), ServiceTiers: []core.ServiceTier{{ID: "priority"}}}
	for _, tc := range []struct {
		effort     *string
		fallback   *string
		canDisable bool
		want       core.ThinkingConfig
		refused    bool
	}{
		{nil, new("high"), false, &core.EffortConfig{Effort: "high"}, false},
		{nil, new("unlisted"), false, &core.EffortConfig{Effort: "low"}, false},
		{nil, nil, true, nil, false}, {new("disabled"), nil, true, &core.DisabledConfig{}, false},
		{new("disabled"), nil, false, nil, true}, {new("ultra"), nil, true, nil, true},
		{new("high"), nil, false, &core.EffortConfig{Effort: "high"}, false},
	} {
		model.CanDisableThinking = &tc.canDisable
		model.DefaultThinkingEffort = tc.fallback
		got, err := model.ThinkingFor(tc.effort)
		if (err != nil) != tc.refused || !reflect.DeepEqual(got, tc.want) {
			t.Errorf("%+v: %v, %v", tc, got, err)
		}
		if tc.refused {
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
	for _, tc := range []struct {
		config core.ThinkingConfig
		want   *string
	}{
		{nil, nil}, {&core.AdaptiveConfig{Effort: "high"}, new("high")}, {&core.EffortConfig{Effort: "low"}, new("low")},
		{&core.DisabledConfig{}, new(core.ThinkingOff)}, {&core.BudgetConfig{BudgetTokens: 1000}, nil},
	} {
		if got := (core.ModelSelection{Thinking: tc.config}).ThinkingEffort(); !reflect.DeepEqual(got, tc.want) {
			t.Errorf("effort: %v", got)
		}
	}
}
