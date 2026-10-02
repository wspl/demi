package anthropicapi

import "github.com/wspl/demi/internal/core"

// directory returns the Claude models offered without a configured catalog.
func directory() core.ProviderModelList {
	wide := []string{"low", "medium", "high", "xhigh", "max"}
	narrow := []string{"low", "medium", "high", "max"}
	defaultID := "claude-opus-4-8"
	return core.ProviderModelList{Models: []core.ProviderModel{
		catalogModel("claude-opus-4-8", "Claude Opus 4.8", 128000, wide),
		catalogModel("claude-opus-4-7", "Claude Opus 4.7", 128000, wide),
		catalogModel("claude-opus-4-6", "Claude Opus 4.6", 128000, narrow),
		catalogModel("claude-sonnet-4-6", "Claude Sonnet 4.6", 64000, narrow),
		catalogModel("claude-fable-5", "Claude Fable 5", 128000, wide),
	}, DefaultModelID: &defaultID, Warnings: []string{}, SourceFetchedAt: "1970-01-01T00:00:00.000Z"}
}

// catalogModel describes a Claude model with a million-token context.
func catalogModel(id, name string, output uint32, efforts []string) core.ProviderModel {
	contextWindow := uint32(1000000)
	yes, no := true, false
	levels := append([]string{}, efforts...)
	return core.ProviderModel{ID: id, DisplayName: name, ContextWindow: &contextWindow, OutputLimit: &output, SupportsTools: &yes, SupportsAttachments: &yes, SupportsVideo: &no, SupportsReasoning: &yes, SupportedThinkingEfforts: &levels, CanDisableThinking: &no, ServiceTiers: []core.ServiceTier{}}
}
