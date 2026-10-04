package google

import "github.com/wspl/demi/internal/types"

func directory() types.ProviderModelList {
	id := "gemini-3.6-flash"
	return types.ProviderModelList{
		Models: []types.ProviderModel{
			model(id, "Gemini 3.6 Flash"),
			model("gemini-3.5-flash", "Gemini 3.5 Flash"),
			model("gemini-3.1-pro-preview", "Gemini 3.1 Pro Preview"),
			model("gemini-2.5-flash", "Gemini 2.5 Flash"),
		},
		DefaultModelID: &id, Warnings: []string{}, SourceFetchedAt: "1970-01-01T00:00:00.000Z",
	}
}

func model(id, name string) types.ProviderModel {
	context, output := uint32(1_048_576), uint32(65_536)
	tools, attachments, video, reasoning := true, true, true, true
	efforts, effort := []string{"low", "medium", "high", "xhigh", "max"}, "medium"
	return types.ProviderModel{
		ID: id, DisplayName: name, ContextWindow: &context, OutputLimit: &output,
		SupportsTools: &tools, SupportsAttachments: &attachments, SupportsVideo: &video,
		SupportsReasoning: &reasoning, SupportedThinkingEfforts: &efforts, DefaultThinkingEffort: &effort,
		ServiceTiers: []types.ServiceTier{},
	}
}
