package codex

import (
	"encoding/json/jsontext"
	"net/http"

	"github.com/wspl/demi/go/provider"
)

//demi:wire
type requestBody struct {
	Model             string                             `json:"model"`
	Instructions      string                             `json:"instructions"`
	Input             []jsontext.Value                   `json:"input"`
	Tools             []provider.ResponsesTool           `json:"tools" check:"each(func=provider.Validate)"`
	ToolChoice        string                             `json:"tool_choice"`
	ParallelToolCalls bool                               `json:"parallel_tool_calls"`
	Store             bool                               `json:"store"`
	Stream            bool                               `json:"stream"`
	Include           []string                           `json:"include"`
	PromptCacheKey    string                             `json:"prompt_cache_key"`
	Text              verbosity                          `json:"text"`
	Reasoning         *provider.ResponsesReasoningConfig `json:"reasoning,omitzero" check:"func=provider.Validate"`
	ServiceTier       *string                            `json:"service_tier,omitzero"`
}

//demi:wire
type verbosity struct {
	Verbosity string `json:"verbosity"`
}

func body(request provider.InferenceRequest) []byte {
	tools := make([]provider.ResponsesTool, 0, len(request.Tools))
	for _, tool := range request.Tools {
		tools = append(tools, provider.NewResponsesTool(tool, true))
	}
	return provider.JSONBody(requestBody{Model: request.ModelID, Instructions: request.SystemPrompt, Input: provider.ResponsesInput(request.Items, provider.ResponsesDialect{SignatureTag: SignatureTag, Assistant: provider.AssistantIdentified, Reasoning: provider.ReasoningWhole, ToolMedia: provider.ToolMediaInline}), Tools: tools, ToolChoice: "auto", ParallelToolCalls: true, Store: false, Stream: true, Include: []string{"reasoning.encrypted_content"}, PromptCacheKey: provider.PromptCacheKey(request.SessionID), Text: verbosity{Verbosity: "low"}, Reasoning: provider.ResponsesReasoning(request.Thinking, true), ServiceTier: request.ServiceTierID})
}
func (p *Provider) inferenceHeaders(secret SecretDocument, request provider.InferenceRequest) http.Header {
	headers := p.accountHeaders(secret)
	headers.Set("Openai-Beta", "responses=experimental")
	headers.Set("Accept", "text/event-stream")
	headers.Set("Content-Type", "application/json")
	session := provider.PromptCacheKey(request.SessionID)
	if provider.HeaderText(session) {
		headers.Set("Session-Id", session)
		headers.Set("Thread-Id", session)
	}
	if provider.HeaderText(request.RequestID) {
		headers.Set("X-Client-Request-Id", request.RequestID)
	}
	return headers
}
