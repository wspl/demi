package openaiapi

import "github.com/wspl/demi/go/core"

func directory() core.ProviderModelList {
	models := []core.ProviderModel{
		model("gpt-5.5", "GPT-5.5", 272000, true, true),
		model("gpt-5.4", "GPT-5.4", 272000, true, true),
		model("gpt-5.4-mini", "GPT-5.4-Mini", 272000, true, false),
		model("gpt-5.3-codex-spark", "GPT-5.3-Codex-Spark", 128000, false, false),
	}
	defaultID := "gpt-5.5"
	return core.ProviderModelList{Models: models, DefaultModelID: &defaultID, Warnings: []string{}, SourceFetchedAt: core.UnixEpoch}
}
func model(id, name string, context uint32, attachments, fast bool) core.ProviderModel {
	yes := true
	efforts := []string{"low", "medium", "high", "xhigh"}
	tiers := []core.ServiceTier{}
	if fast {
		description := "1.5x speed, increased usage"
		tiers = append(tiers, core.ServiceTier{ID: "priority", Label: "Fast", Description: &description, Fast: true})
	}
	return core.ProviderModel{ID: id, DisplayName: name, ContextWindow: &context, SupportsTools: &yes, SupportsAttachments: &attachments, SupportsReasoning: &yes, SupportedThinkingEfforts: &efforts, ServiceTiers: tiers}
}
