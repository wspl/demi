package providers_test

import (
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

func TestEveryRequestTakesConfiguredFactsAndKeepsUserChoices(t *testing.T) {
	models, err := webapi.DecodeConfiguredModels(
		[]byte(
			`[{"id":"gpt-5.5","displayName":"GPT-5.5","contextWindow":272000,` +
				`"outputLimit":4000,"thinkingEfforts":["low","high"],` +
				`"acceptedExtensions":["png","pdf"],"fastTier":"priority"}]`,
		),
	)
	if err != nil {
		t.Fatal(err)
	}
	entry := providers.Entry{ID: "entry-1", Credential: &providers.APIKeyConfig{Models: &models}}
	output := uint32(100)
	tier := "priority"
	chosen := core.ModelSelection{
		ProviderID: "entry-1",
		Model: core.Model{
			ID:            "gpt-5.5",
			Name:          "stale",
			ContextWindow: 1000,
			OutputLimit:   &output,
		},
		Thinking:      &core.EffortConfig{Effort: "high"},
		ServiceTierID: &tier,
	}
	applied, err := providers.ConfiguredSelection(entry, chosen)
	if err != nil {
		t.Fatal(err)
	}
	if applied.Model.Name != "GPT-5.5" || applied.Model.ContextWindow != 272000 || *applied.Model.OutputLimit != 4000 {
		t.Fatalf("facts: %+v", applied.Model)
	}
	if diff := cmp.Diff([]core.FileExtension{"png", "pdf"}, *applied.Model.AcceptedExtensions); diff != "" {
		t.Fatal(diff)
	}
	if diff := cmp.Diff(chosen.Thinking, applied.Thinking); diff != "" {
		t.Fatal(diff)
	}
	if *applied.ServiceTierID != tier {
		t.Fatal("lost tier")
	}
	edited := uint32(8000)
	models[0].OutputLimit = &edited
	applied, err = providers.ConfiguredSelection(entry, chosen)
	if err != nil || *applied.Model.OutputLimit != 8000 {
		t.Fatalf("edit: %v %v", applied, err)
	}
	other := chosen
	other.Model.ID = "gpt-4"
	if _, err := providers.ConfiguredSelection(
		entry,
		other,
	); err == nil ||
		err.Error() != "the model gpt-4 is not in the provider's configured list" {
		t.Fatal(err)
	}
	entry.Credential = &providers.APIKeyConfig{}
	applied, err = providers.ConfiguredSelection(entry, chosen)
	if err != nil {
		t.Fatal(err)
	}
	if diff := cmp.Diff(chosen, applied); diff != "" {
		t.Fatal(diff)
	}
}
