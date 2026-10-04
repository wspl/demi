package openaiapi

import (
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

func responsesBody(request provider.InferenceRequest, policy provider.VendorPolicy) ([]byte, error) {
	assistant := provider.AssistantMinimal
	if policy.ReplayAssistantStatus {
		assistant = provider.AssistantCompleted
	}
	input, err := provider.ResponsesInput(request.Items, provider.ResponsesDialect{
		SignatureTag: signatureTag,
		Assistant:    assistant,
		Reasoning:    provider.ReasoningReplayable,
		ToolMedia:    provider.ToolMediaFollowUp,
	})
	if err != nil {
		return nil, err
	}
	body := responsesRequest{
		Model:           request.ModelID,
		Input:           input,
		Stream:          true,
		Store:           false,
		Include:         [1]string{"reasoning.encrypted_content"},
		PromptCacheKey:  provider.PromptCacheKey(request.SessionID),
		MaxOutputTokens: request.MaxOutputTokens(),
		Reasoning:       provider.ResponsesReasoning(request.Thinking, provider.SummaryOmitted),
		ServiceTier:     request.ServiceTierID,
	}
	if !types.IsBlank(request.SystemPrompt) {
		body.Instructions = &request.SystemPrompt
	}
	for _, tool := range request.Tools {
		body.Tools = append(body.Tools, provider.NewResponsesTool(tool, false))
	}
	if len(body.Tools) > 0 {
		choice := "auto"
		parallel := true
		body.ToolChoice = &choice
		body.ParallelToolCalls = &parallel
	}
	return provider.JSONBody(body)
}

type responsesRequest struct {
	Model             string                   `json:"model"`
	Input             []provider.InputItem     `json:"input"`
	Stream            bool                     `json:"stream"`
	Store             bool                     `json:"store"`
	Include           [1]string                `json:"include"`
	PromptCacheKey    string                   `json:"prompt_cache_key"`
	MaxOutputTokens   *uint32                  `json:"max_output_tokens,omitempty"`
	Instructions      *string                  `json:"instructions,omitempty"`
	Tools             []provider.ResponsesTool `json:"tools,omitempty"`
	ToolChoice        *string                  `json:"tool_choice,omitempty"`
	ParallelToolCalls *bool                    `json:"parallel_tool_calls,omitempty"`
	Reasoning         *provider.Reasoning      `json:"reasoning,omitempty"`
	ServiceTier       *string                  `json:"service_tier,omitempty"`
}

func chatBody(request provider.InferenceRequest, policy provider.VendorPolicy) ([]byte, error) {
	messages, err := provider.ChatMessages(
		request.SystemPrompt,
		request.Items,
		provider.ChatDialect{ReasoningContent: policy.PassBackReasoningContent, Media: provider.ChatNative},
	)
	if err != nil {
		return nil, err
	}
	body := chatRequest{
		Model: request.ModelID, Messages: messages, Stream: true, MaxCompletionTokens: request.MaxOutputTokens(),
		ReasoningEffort: provider.ReasoningEffort(request.Thinking), ServiceTier: request.ServiceTierID,
		StreamOptions: streamOptions{IncludeUsage: true},
	}
	for _, tool := range request.Tools {
		body.Tools = append(body.Tools, provider.NewChatTool(tool))
	}
	if len(body.Tools) > 0 {
		choice := "auto"
		body.ToolChoice = &choice
	}
	return provider.JSONBody(body)
}

type chatRequest struct {
	Model               string                 `json:"model"`
	Messages            []provider.ChatMessage `json:"messages"`
	Stream              bool                   `json:"stream"`
	Tools               []provider.ChatTool    `json:"tools,omitempty"`
	ToolChoice          *string                `json:"tool_choice,omitempty"`
	MaxCompletionTokens *uint32                `json:"max_completion_tokens,omitempty"`
	ReasoningEffort     *string                `json:"reasoning_effort,omitempty"`
	ServiceTier         *string                `json:"service_tier,omitempty"`
	StreamOptions       streamOptions          `json:"stream_options"`
}

type streamOptions struct {
	IncludeUsage bool `json:"include_usage"`
}
