package providerhost_test

import (
	"testing"

	"github.com/google/go-cmp/cmp"
	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

func TestEveryRequestTakesConfiguredFactsAndKeepsUserChoices(t *testing.T) {
	models, err := webapiproto.DecodeConfiguredModels(
		[]byte(
			`[{"id":"gpt-5.5","displayName":"GPT-5.5","contextWindow":272000,` +
				`"outputLimit":4000,"thinkingEfforts":["low","high"],` +
				`"acceptedExtensions":["png","pdf"],"fastTier":"priority"}]`,
		),
	)
	if err != nil {
		t.Fatal(err)
	}
	entry := providerhost.Entry{ID: "entry-1", Credential: &providerhost.APIKeyConfig{Models: &models}}
	output := uint32(100)
	tier := "priority"
	chosen := types.ModelSelection{
		ProviderID: "entry-1",
		Model: types.Model{
			ID:            "gpt-5.5",
			Name:          "stale",
			ContextWindow: 1000,
			OutputLimit:   &output,
		},
		Thinking:      &types.EffortConfig{Effort: "high"},
		ServiceTierID: &tier,
	}
	applied, err := providerhost.ConfiguredSelection(entry, chosen)
	if err != nil {
		t.Fatal(err)
	}
	if applied.Model.Name != "GPT-5.5" || applied.Model.ContextWindow != 272000 || *applied.Model.OutputLimit != 4000 {
		t.Fatalf("facts: %+v", applied.Model)
	}
	if diff := cmp.Diff([]types.FileExtension{"png", "pdf"}, *applied.Model.AcceptedExtensions); diff != "" {
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
	applied, err = providerhost.ConfiguredSelection(entry, chosen)
	if err != nil || *applied.Model.OutputLimit != 8000 {
		t.Fatalf("edit: %v %v", applied, err)
	}
	other := chosen
	other.Model.ID = "gpt-4"
	if _, err := providerhost.ConfiguredSelection(
		entry,
		other,
	); err == nil ||
		err.Error() != "the model gpt-4 is not in the provider's configured list" {
		t.Fatal(err)
	}
	entry.Credential = &providerhost.APIKeyConfig{}
	applied, err = providerhost.ConfiguredSelection(entry, chosen)
	if err != nil {
		t.Fatal(err)
	}
	if diff := cmp.Diff(chosen, applied); diff != "" {
		t.Fatal(diff)
	}
}
