package openaiapi

import "github.com/wspl/demi/internal/core"

func directory() core.ProviderModelList {
	defaultID := "gpt-5.5"
	return core.ProviderModelList{
		Models: []core.ProviderModel{
			model("gpt-5.5", "GPT-5.5", 272_000, true, true),
			model("gpt-5.4", "GPT-5.4", 272_000, true, true),
			model("gpt-5.4-mini", "GPT-5.4-Mini", 272_000, true, false),
			model("gpt-5.3-codex-spark", "GPT-5.3-Codex-Spark", 128_000, false, false),
		},
		DefaultModelID: &defaultID, Warnings: []string{}, SourceFetchedAt: "1970-01-01T00:00:00.000Z",
	}
}

// model describes a GPT model's reasoning, attachment and Fast-tier support.
func model(id, name string, contextWindow uint32, attachments, fast bool) core.ProviderModel {
	efforts := []string{"low", "medium", "high", "xhigh"}
	supported := true
	tiers := []core.ServiceTier{}
	if fast {
		description := "1.5x speed, increased usage"
		tiers = append(tiers, core.ServiceTier{ID: "priority", Label: "Fast", Description: &description, Fast: true})
	}
	return core.ProviderModel{
		ID:                       id,
		DisplayName:              name,
		ContextWindow:            &contextWindow,
		SupportsTools:            &supported,
		SupportsAttachments:      &attachments,
		SupportsReasoning:        &supported,
		SupportedThinkingEfforts: &efforts,
		ServiceTiers:             tiers,
	}
}
