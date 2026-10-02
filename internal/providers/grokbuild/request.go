package grokbuild

import (
	"net/http"

	"github.com/wspl/demi/internal/provider"
)

const clientVersion = "1.0.5"

func encodeRequest(r provider.InferenceRequest) ([]byte, error) {
	messages, err := provider.ChatMessages(r.SystemPrompt, r.Items, provider.ChatDialect{Media: provider.ChatImages})
	if err != nil {
		return nil, err
	}
	body := struct {
		Model         string                 `json:"model"`
		Messages      []provider.ChatMessage `json:"messages"`
		Stream        bool                   `json:"stream"`
		StreamOptions struct {
			IncludeUsage bool `json:"include_usage"`
		} `json:"stream_options"`
		Tools      []provider.ChatTool `json:"tools,omitempty"`
		ToolChoice *string             `json:"tool_choice,omitempty"`
		Effort     *string             `json:"reasoning_effort,omitempty"`
	}{Model: r.ModelID, Messages: messages, Stream: true, Effort: provider.ReasoningEffort(r.Thinking)}
	body.StreamOptions.IncludeUsage = true
	for _, tool := range r.Tools {
		body.Tools = append(body.Tools, provider.NewChatTool(tool))
	}
	if len(body.Tools) > 0 {
		choice := "auto"
		body.ToolChoice = &choice
	}
	return provider.JSONBody(body)
}

// identityHeaders carries the CLI identity used by Grok's proxy.
func identityHeaders(s secret) http.Header {
	h := http.Header{}
	h.Set("Authorization", s.AccessToken.Bearer().Expose())
	h.Set("x-xai-token-auth", "xai-grok-cli")
	h.Set("x-authenticateresponse", "authenticate-response")
	h.Set("x-grok-client-version", clientVersion)
	h.Set("x-grok-client-identifier", "grok-shell")
	h.Set("x-grok-client-mode", "interactive")
	if s.UserID != nil {
		setIdentity(h, "x-userid", *s.UserID)
		setIdentity(h, "x-grok-user-id", *s.UserID)
	}
	if s.Email != nil {
		setIdentity(h, "x-email", *s.Email)
	}
	return h
}

// setIdentity omits identity text HTTP cannot carry, as the token still identifies the caller.
func setIdentity(h http.Header, name, value string) {
	for i := 0; i < len(value); i++ {
		if (value[i] < 32 && value[i] != '\t') || value[i] == 127 {
			return
		}
	}
	h.Set(name, value)
}
func inferenceHeaders(s secret, r provider.InferenceRequest) http.Header {
	h := identityHeaders(s)
	for name, value := range map[string]string{"x-grok-model-override": r.ModelID, "x-grok-session-id": r.SessionID, "x-grok-conv-id": r.SessionID, "x-grok-req-id": r.RequestID, "x-grok-turn-idx": r.TurnID} {
		setIdentity(h, name, value)
	}
	h.Set("Accept", "text/event-stream")
	h.Set("Content-Type", "application/json")
	return h
}
