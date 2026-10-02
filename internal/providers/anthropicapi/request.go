package anthropicapi

import (
	"encoding/base64"
	"encoding/json"
	"strings"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

const signatureTag = "anthropic:"

type block map[string]any
type message struct {
	Role    string  `json:"role"`
	Content []block `json:"content"`
}
type requestBody struct {
	Model        string    `json:"model"`
	Messages     []message `json:"messages"`
	MaxTokens    uint32    `json:"max_tokens"`
	Stream       bool      `json:"stream"`
	System       []block   `json:"system,omitempty"`
	Tools        []block   `json:"tools,omitempty"`
	Thinking     block     `json:"thinking,omitempty"`
	OutputConfig block     `json:"output_config,omitempty"`
	ServiceTier  *string   `json:"service_tier,omitempty"`
}

// encodeRequest maps the transcript and cache boundaries onto a Messages request.
func encodeRequest(request provider.InferenceRequest, policy provider.VendorPolicy) ([]byte, error) {
	body := requestBody{Model: request.ModelID, Messages: []message{}, MaxTokens: 32000, Stream: true, ServiceTier: request.ServiceTierID}
	if limit := request.MaxOutputTokens(); limit != nil {
		body.MaxTokens = *limit
	}
	body.Thinking, body.OutputConfig = thinking(request.Thinking, body.MaxTokens, policy)
	if !core.IsBlank(request.SystemPrompt) {
		body.System = []block{textBlock(request.SystemPrompt)}
	}
	for _, tool := range request.Tools {
		body.Tools = append(body.Tools, block{"name": tool.Name, "description": tool.Description, "input_schema": tool.InputSchema})
	}
	var answered block
	var lastMarkable block
	for index, item := range request.Items {
		role, content := itemContent(item)
		if len(content) > 0 {
			end := len(body.Messages) - 1
			if end >= 0 && body.Messages[end].Role == role {
				body.Messages[end].Content = append(body.Messages[end].Content, content...)
			} else {
				body.Messages = append(body.Messages, message{Role: role, Content: content})
			}
			for _, b := range content {
				if b["type"] != "thinking" && b["type"] != "redacted_thinking" {
					lastMarkable = b
				}
			}
		}
		if request.PromptCache.AnsweredItems != nil && index+1 == *request.PromptCache.AnsweredItems {
			answered = lastMarkable
		}
	}
	if request.PromptCache.AnsweredItems != nil {
		if len(body.System) > 0 {
			mark(body.System[len(body.System)-1])
		} else if len(body.Tools) > 0 {
			mark(body.Tools[len(body.Tools)-1])
		}
		if answered != nil {
			mark(answered)
		}
		if lastMarkable != nil {
			mark(lastMarkable)
		}
	}
	return provider.JSONBody(body)
}

// mark gives a vendor cache boundary a one-hour lifetime.
func mark(b block) { b["cache_control"] = block{"type": "ephemeral", "ttl": "1h"} }

// thinking converts configured reasoning into the vendor's budget or effort.
func thinking(config core.ThinkingConfig, maxTokens uint32, policy provider.VendorPolicy) (block, block) {
	var budget uint32
	var effort string
	isBudget := false
	display := "summarized"
	switch c := config.(type) {
	case nil, *core.DisabledConfig:
		return nil, nil
	case *core.BudgetConfig:
		budget = c.BudgetTokens
		isBudget = true
	case *core.AdaptiveConfig:
		effort = c.Effort
	case *core.EffortConfig:
		effort = c.Effort
		if c.Summary != nil && *c.Summary == "off" {
			display = "omitted"
		}
	}
	if !isBudget && !policy.EffortAsBudget {
		return block{"type": "adaptive", "display": display}, block{"effort": effort}
	}
	if !isBudget {
		budget = provider.EffortBudget(effort)
	}
	ceiling := uint32(1024)
	if maxTokens > 2048 {
		ceiling = maxTokens - 1024
	}
	return block{"type": "enabled", "budget_tokens": max(uint32(1024), min(budget, ceiling))}, nil
}

// itemContent maps one transcript item, omitting reasoning that cannot be replayed.
func itemContent(item provider.InferenceItem) (string, []block) {
	switch item := item.(type) {
	case *provider.UserMessage:
		return "user", userContent(item.Content)
	case *provider.UserSteer:
		return "user", userContent(item.Content)
	case *provider.AssistantText:
		return "assistant", []block{textBlock(item.Text)}
	case *provider.AssistantThinking:
		if item.KeptPastSummary || item.Signature == nil {
			return "", nil
		}
		signature, ok := strings.CutPrefix(*item.Signature, signatureTag)
		if !ok {
			return "", nil
		}
		return "assistant", []block{{"type": "thinking", "thinking": item.Text, "signature": signature}}
	case *provider.AssistantRedactedThinking:
		data, ok := strings.CutPrefix(item.Data, signatureTag)
		if item.KeptPastSummary || !ok {
			return "", nil
		}
		return "assistant", []block{{"type": "redacted_thinking", "data": data}}
	case *provider.ToolUse:
		input := item.Input
		if len(input) == 0 || strings.TrimSpace(string(input)) == "null" {
			input = json.RawMessage(`{}`)
		}
		return "assistant", []block{{"type": "tool_use", "id": item.ToolUseID, "name": item.ToolName, "input": input}}
	case *provider.ToolResult:
		b := block{"type": "tool_result", "tool_use_id": item.ToolUseID, "content": resultContent(item.Output)}
		if item.IsError {
			b["is_error"] = true
		}
		return "user", []block{b}
	}
	return "", nil
}

// textBlock constructs a Messages text block.
func textBlock(text string) block { return block{"type": "text", "text": text} }

// source constructs the vendor's inline base64 source.
func source(bytes provider.MediaBytes) block {
	return block{"type": "base64", "media_type": bytes.MediaType, "data": base64.StdEncoding.EncodeToString(bytes.Data)}
}

// userContent maps native media and the API's unsupported-media placeholders.
func userContent(parts []provider.UserPart) []block {
	out := make([]block, 0, len(parts))
	for _, part := range parts {
		switch p := part.(type) {
		case *provider.TextPart:
			out = append(out, textBlock(p.Text))
		case *provider.VideoPart:
			out = append(out, textBlock("[video]"))
		case *provider.ImagePart:
			switch medium := p.Medium.(type) {
			case *provider.MediaBytes:
				out = append(out, block{"type": "image", "source": source(*medium)})
			case *provider.MediaURL:
				out = append(out, textBlock("[image:"+medium.URL+"]"))
			}
		case *provider.DocumentPart:
			out = append(out, block{"type": "document", "source": source(p.Bytes), "title": p.FileName})
		}
	}
	return out
}

// resultContent maps a tool result's text and media onto Messages blocks.
func resultContent(parts []provider.ResultPart) []block {
	out := make([]block, 0, len(parts))
	for _, part := range parts {
		switch p := part.(type) {
		case *provider.TextPart:
			out = append(out, textBlock(p.Text))
		case *provider.ResultImage:
			out = append(out, block{"type": "image", "source": source(p.Bytes)})
		case *provider.ResultVideo:
			out = append(out, textBlock("[video:"+p.Bytes.MediaType+"]"))
		}
	}
	return out
}
