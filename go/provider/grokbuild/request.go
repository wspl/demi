package grokbuild

import (
	"encoding/json/jsontext"
	"net/http"

	"github.com/wspl/demi/go/provider"
)

//demi:wire
type requestBody struct {
	Model           string               `json:"model"`
	Messages        []jsontext.Value     `json:"messages"`
	Stream          bool                 `json:"stream"`
	StreamOptions   streamOptions        `json:"stream_options"`
	Tools           *[]provider.ChatTool `json:"tools,omitzero" check:"each(func=provider.Validate)"`
	ToolChoice      *string              `json:"tool_choice,omitzero"`
	ReasoningEffort *string              `json:"reasoning_effort,omitzero"`
}

//demi:wire
type streamOptions struct {
	IncludeUsage bool `json:"include_usage"`
}

func encodeRequest(request provider.InferenceRequest) []byte {
	body := requestBody{Model: request.ModelID, Messages: provider.ChatMessages(request.SystemPrompt, request.Items, provider.ChatDialect{ImagesOnly: true}), Stream: true, StreamOptions: streamOptions{IncludeUsage: true}, ReasoningEffort: provider.ReasoningEffort(request.Thinking)}
	if len(request.Tools) > 0 {
		tools := make([]provider.ChatTool, 0, len(request.Tools))
		for _, tool := range request.Tools {
			tools = append(tools, provider.NewChatTool(tool))
		}
		body.Tools = &tools
		body.ToolChoice = new("auto")
	}
	return provider.JSONBody(body)
}

func identityHeaders(secret SecretDocument) http.Header {
	headers := http.Header{"Authorization": {secret.AccessToken.Bearer()}, "X-Xai-Token-Auth": {"xai-grok-cli"}, "X-Authenticateresponse": {"authenticate-response"}, "X-Grok-Client-Version": {ClientVersion}, "X-Grok-Client-Identifier": {"grok-shell"}, "X-Grok-Client-Mode": {"interactive"}}
	if secret.UserID != nil && provider.HeaderText(*secret.UserID) {
		headers.Set("X-Userid", *secret.UserID)
		headers.Set("X-Grok-User-Id", *secret.UserID)
	}
	if secret.Email != nil && provider.HeaderText(*secret.Email) {
		headers.Set("X-Email", *secret.Email)
	}
	return headers
}
func inferenceHeaders(secret SecretDocument, request provider.InferenceRequest) http.Header {
	headers := identityHeaders(secret)
	for name, value := range map[string]string{"X-Grok-Model-Override": request.ModelID, "X-Grok-Session-Id": request.SessionID, "X-Grok-Conv-Id": request.SessionID, "X-Grok-Req-Id": request.RequestID, "X-Grok-Turn-Idx": request.TurnID} {
		if provider.HeaderText(value) {
			headers.Set(name, value)
		}
	}
	headers.Set("Accept", "text/event-stream")
	headers.Set("Content-Type", "application/json")
	return headers
}
