package openaiapi

import (
	"encoding/json/jsontext"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

//demi:wire
type responsesRequest struct {
	Model             string                             `json:"model"`
	Input             []jsontext.Value                   `json:"input"`
	Stream            bool                               `json:"stream"`
	Store             bool                               `json:"store"`
	Include           []string                           `json:"include"`
	PromptCacheKey    string                             `json:"prompt_cache_key"`
	MaxOutputTokens   *uint32                            `json:"max_output_tokens,omitzero"`
	Instructions      *string                            `json:"instructions,omitzero"`
	Tools             *[]provider.ResponsesTool          `json:"tools,omitzero" check:"each(func=provider.Validate)"`
	ToolChoice        *string                            `json:"tool_choice,omitzero"`
	ParallelToolCalls *bool                              `json:"parallel_tool_calls,omitzero"`
	Reasoning         *provider.ResponsesReasoningConfig `json:"reasoning,omitzero" check:"func=provider.Validate"`
	ServiceTier       *string                            `json:"service_tier,omitzero"`
}

func responsesBody(request provider.InferenceRequest, policy VendorPolicy) []byte {
	replay := provider.AssistantMinimal
	if policy.ReplayAssistantStatus {
		replay = provider.AssistantCompleted
	}
	body := responsesRequest{Model: request.ModelID, Input: provider.ResponsesInput(request.Items, provider.ResponsesDialect{SignatureTag: SignatureTag, Assistant: replay, Reasoning: provider.ReasoningReplayable, ToolMedia: provider.ToolMediaFollowUp}), Stream: true, Store: false, Include: []string{"reasoning.encrypted_content"}, PromptCacheKey: provider.PromptCacheKey(request.SessionID), MaxOutputTokens: request.MaxOutputTokens(), Reasoning: provider.ResponsesReasoning(request.Thinking, false), ServiceTier: request.ServiceTierID}
	if !core.IsBlank(request.SystemPrompt) {
		body.Instructions = &request.SystemPrompt
	}
	if len(request.Tools) > 0 {
		tools := make([]provider.ResponsesTool, 0, len(request.Tools))
		for _, tool := range request.Tools {
			tools = append(tools, provider.NewResponsesTool(tool, false))
		}
		body.Tools = &tools
		choice, parallel := "auto", true
		body.ToolChoice = &choice
		body.ParallelToolCalls = &parallel
	}
	return provider.JSONBody(body)
}

//demi:wire
type chatRequest struct {
	Model               string               `json:"model"`
	Messages            []jsontext.Value     `json:"messages"`
	Stream              bool                 `json:"stream"`
	Tools               *[]provider.ChatTool `json:"tools,omitzero" check:"each(func=provider.Validate)"`
	ToolChoice          *string              `json:"tool_choice,omitzero"`
	MaxCompletionTokens *uint32              `json:"max_completion_tokens,omitzero"`
	ReasoningEffort     *string              `json:"reasoning_effort,omitzero"`
	ServiceTier         *string              `json:"service_tier,omitzero"`
	StreamOptions       streamOptions        `json:"stream_options"`
}

//demi:wire
type streamOptions struct {
	IncludeUsage bool `json:"include_usage"`
}

func chatBody(request provider.InferenceRequest, policy VendorPolicy) []byte {
	body := chatRequest{Model: request.ModelID, Messages: provider.ChatMessages(request.SystemPrompt, request.Items, provider.ChatDialect{ReasoningContent: policy.PassBackReasoningContent}), Stream: true, MaxCompletionTokens: request.MaxOutputTokens(), ReasoningEffort: provider.ReasoningEffort(request.Thinking), ServiceTier: request.ServiceTierID, StreamOptions: streamOptions{IncludeUsage: true}}
	if len(request.Tools) > 0 {
		tools := make([]provider.ChatTool, 0, len(request.Tools))
		for _, tool := range request.Tools {
			tools = append(tools, provider.NewChatTool(tool))
		}
		body.Tools = &tools
		choice := "auto"
		body.ToolChoice = &choice
	}
	return provider.JSONBody(body)
}
