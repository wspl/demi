package core_test

import (
	"reflect"
	"testing"

	"github.com/wspl/demi/go/core"
)

func TestCatalogSelection(t *testing.T) {
	for _, test := range []struct {
		attachments, video *bool
		exact, want        *[]core.FileExtension
	}{
		{nil, nil, nil, nil}, {nil, new(false), nil, nil},
		{new(false), nil, nil, new([]core.FileExtension{})},
		{new(true), nil, nil, &core.AttachmentFileExtensions},
		{new(true), new(false), nil, &core.AttachmentFileExtensions},
		{new(true), new(true), nil, new(append(append([]core.FileExtension{}, core.AttachmentFileExtensions...), core.VideoFileExtensions...))},
		{new(false), new(true), nil, &core.VideoFileExtensions}, {nil, new(true), nil, &core.VideoFileExtensions},
		{new(true), nil, new([]core.FileExtension{core.FileExtensionPng}), new([]core.FileExtension{core.FileExtensionPng})},
		{new(true), new(true), new([]core.FileExtension{}), new([]core.FileExtension{})},
	} {
		model := core.ProviderModel{ID: "model", DisplayName: "Model", ContextWindow: new(uint32(200000)), OutputLimit: new(uint32(8000)), SupportsAttachments: test.attachments, SupportsVideo: test.video, AcceptedExtensions: test.exact}
		var thinking core.ThinkingConfig = core.ThinkingConfigEffort{Effort: "high"}
		selection := model.Selection("provider", &thinking, new("priority"))
		if selection.ProviderID != "provider" || selection.Model.ID != "model" || selection.Model.Name != "Model" || selection.Model.ContextWindow != 200000 || *selection.Model.OutputLimit != 8000 || selection.Model.InputLimit != nil || *selection.ServiceTierID != "priority" || *selection.ThinkingEffort() != "high" {
			t.Fatal("selection lost facts")
		}
		*model.OutputLimit = 1
		if *selection.Model.OutputLimit != 8000 {
			t.Fatal("catalog refresh changed saved selection")
		}
		if !reflect.DeepEqual(selection.Model.AcceptedExtensions, test.want) {
			t.Errorf("accepted extensions: got %v want %v", selection.Model.AcceptedExtensions, test.want)
		}
	}
	plain := (core.ProviderModel{ID: "m"}).Selection("p", nil, nil)
	if plain.Model.ContextWindow != 0 || plain.Thinking != nil || plain.ServiceTierID != nil {
		t.Fatal("unknown defaults changed")
	}
}

func TestCatalogThinkingAndTier(t *testing.T) {
	for _, test := range []struct {
		reasoning *bool
		efforts   *[]string
		count     int
		disabled  bool
	}{
		{new(false), new([]string{"low"}), 1, true}, {nil, nil, 0, false}, {new(true), new([]string{}), 0, false}, {new(true), new([]string{"low", "high"}), 1, false},
	} {
		model := core.ProviderModel{SupportsReasoning: test.reasoning, SupportedThinkingEfforts: test.efforts, DefaultThinkingEffort: new("high")}
		choices := model.Selection("p", nil, nil).Model.Thinking
		if len(choices) != test.count {
			t.Fatal("thinking choices", choices)
		}
		if test.count == 1 {
			if test.disabled {
				if _, ok := choices[0].(core.ThinkingCapabilityDisabled); !ok {
					t.Fatal(choices)
				}
			} else {
				effort := choices[0].(core.ThinkingCapabilityEffort)
				if !reflect.DeepEqual(effort.Efforts, []string{"low", "high"}) || *effort.DefaultEffort != "high" || !reflect.DeepEqual(effort.Summaries, []core.ThinkingSummary{core.ThinkingSummaryAuto, core.ThinkingSummaryConcise, core.ThinkingSummaryDetailed, core.ThinkingSummaryOff, core.ThinkingSummaryOn}) || effort.DefaultSummary != nil {
					t.Fatal(effort)
				}
			}
		}
	}
	model := core.ProviderModel{SupportedThinkingEfforts: new([]string{"low", "high"}), DefaultThinkingEffort: new("high"), CanDisableThinking: new(false), ServiceTiers: []core.ServiceTier{{ID: "priority"}}}
	thinking, err := model.ThinkingFor(nil)
	if err != nil || *(core.ModelSelection{Thinking: thinking}).ThinkingEffort() != "high" {
		t.Fatal("required thinking default", err)
	}
	model.DefaultThinkingEffort = new("absent")
	if *model.UnnamedEffort() != "low" {
		t.Fatal("default not in list")
	}
	if _, err := model.ThinkingFor(new(core.ThinkingOff)); err == nil {
		t.Fatal("disabled mandatory thinking")
	}
	model.CanDisableThinking = new(true)
	if got, err := model.ThinkingFor(nil); err != nil || got != nil {
		t.Fatal("optional thinking default", err)
	}
	thinking, err = model.ThinkingFor(new(core.ThinkingOff))
	if err != nil || *(core.ModelSelection{Thinking: thinking}).ThinkingEffort() != core.ThinkingOff {
		t.Fatal("off", err)
	}
	if _, err := model.ThinkingFor(new("missing")); err == nil {
		t.Fatal("unknown effort")
	}
	if got, err := model.TierFor(new("priority")); err != nil || *got != "priority" {
		t.Fatal("tier", err)
	}
	if got, err := model.TierFor(nil); err != nil || got != nil {
		t.Fatal("default tier", err)
	}
	if _, err := model.TierFor(new("missing")); err == nil {
		t.Fatal("unknown tier")
	}
}
