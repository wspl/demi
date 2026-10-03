package codex

import "github.com/wspl/demi/internal/provider"

type requestBody struct {
	Model             string                   `json:"model"`
	Instructions      string                   `json:"instructions"`
	Input             []provider.InputItem     `json:"input"`
	Tools             []provider.ResponsesTool `json:"tools"`
	ToolChoice        string                   `json:"tool_choice"`
	ParallelToolCalls bool                     `json:"parallel_tool_calls"`
	Store             bool                     `json:"store"`
	Stream            bool                     `json:"stream"`
	Include           []string                 `json:"include"`
	PromptCacheKey    string                   `json:"prompt_cache_key"`
	Text              requestText              `json:"text"`
	Reasoning         *provider.Reasoning      `json:"reasoning,omitempty"`
	ServiceTier       *string                  `json:"service_tier,omitempty"`
}
type requestText struct {
	Verbosity string `json:"verbosity"`
}

func encodeRequest(r provider.InferenceRequest) ([]byte, error) {
	input, err := provider.ResponsesInput(
		r.Items,
		provider.ResponsesDialect{
			SignatureTag: "codex:",
			Assistant:    provider.AssistantIdentified,
			Reasoning:    provider.ReasoningWhole,
			ToolMedia:    provider.ToolMediaInline,
		},
	)
	if err != nil {
		return nil, err
	}
	tools := make([]provider.ResponsesTool, 0, len(r.Tools))
	for _, tool := range r.Tools {
		tools = append(tools, provider.NewResponsesTool(tool, true))
	}
	return provider.JSONBody(
		requestBody{
			Model:             r.ModelID,
			Instructions:      r.SystemPrompt,
			Input:             input,
			Tools:             tools,
			ToolChoice:        "auto",
			ParallelToolCalls: true,
			Stream:            true,
			Include:           []string{"reasoning.encrypted_content"},
			PromptCacheKey:    provider.PromptCacheKey(r.SessionID),
			Text:              requestText{Verbosity: "low"},
			Reasoning:         provider.ResponsesReasoning(r.Thinking, provider.SummaryAuto),
			ServiceTier:       r.ServiceTierID,
		},
	)
}
