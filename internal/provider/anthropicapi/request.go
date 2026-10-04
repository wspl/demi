package anthropicapi

import (
	"encoding/base64"
	"encoding/json"
	"strings"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

const signatureTag = "anthropic:"

// encodeRequest maps the transcript and cache boundaries onto a Messages request.
func encodeRequest(request provider.InferenceRequest, policy provider.VendorPolicy) ([]byte, error) {
	body := requestBody{
		Model:       request.ModelID,
		Messages:    []message{},
		MaxTokens:   32000,
		Stream:      true,
		ServiceTier: request.ServiceTierID,
	}
	if limit := request.MaxOutputTokens(); limit != nil {
		body.MaxTokens = *limit
	}
	body.Thinking, body.OutputConfig = thinking(request.Thinking, body.MaxTokens, policy)
	if !types.IsBlank(request.SystemPrompt) {
		body.System = []*content{{block: newTextBlock(request.SystemPrompt)}}
	}
	for _, definition := range request.Tools {
		body.Tools = append(
			body.Tools,
			tool{Name: definition.Name, Description: definition.Description, InputSchema: definition.InputSchema},
		)
	}
	var answered *content
	var lastMarkable *content
	for index, item := range request.Items {
		role, blocks := itemContent(item)
		contents := make([]*content, 0, len(blocks))
		for _, b := range blocks {
			contents = append(contents, &content{block: b})
		}
		if len(contents) > 0 {
			end := len(body.Messages) - 1
			if end >= 0 && body.Messages[end].Role == role {
				body.Messages[end].Content = append(body.Messages[end].Content, contents...)
			} else {
				body.Messages = append(body.Messages, message{Role: role, Content: contents})
			}
			for _, b := range contents {
				if b.canMark() {
					lastMarkable = b
				}
			}
		}
		if request.PromptCache.AnsweredItems != nil && index+1 == *request.PromptCache.AnsweredItems {
			answered = lastMarkable
		}
	}
	if request.PromptCache.AnsweredItems != nil {
		markRequestCache(&body, answered, lastMarkable)
	}
	return provider.JSONBody(body)
}

// mark gives a vendor cache boundary a one-hour lifetime.
func mark(c *content) {
	c.cache = &cacheControl{Type: "ephemeral", TTL: "1h"}
}

// thinking converts configured reasoning into the vendor's budget or effort.
func thinking(
	config types.ThinkingConfig,
	maxTokens uint32,
	policy provider.VendorPolicy,
) (thinkingConfig, *outputConfig) {
	var budget uint32
	var effort string
	isBudget := false
	display := "summarized"
	switch c := config.(type) {
	case nil, *types.DisabledConfig:
		return nil, nil
	case *types.BudgetConfig:
		budget = c.BudgetTokens
		isBudget = true
	case *types.AdaptiveConfig:
		effort = c.Effort
	case *types.EffortConfig:
		effort = c.Effort
		if c.Summary != nil && *c.Summary == "off" {
			display = "omitted"
		}
	}
	if !isBudget && !policy.EffortAsBudget {
		return &adaptiveThinking{Type: "adaptive", Display: display}, &outputConfig{Effort: effort}
	}
	if !isBudget {
		budget = provider.EffortBudget(effort)
	}
	ceiling := uint32(1024)
	if maxTokens > 2048 {
		ceiling = maxTokens - 1024
	}
	return &enabledThinking{Type: "enabled", BudgetTokens: max(uint32(1024), min(budget, ceiling))}, nil
}

// itemContent maps one transcript item, omitting reasoning that cannot be replayed.
func itemContent(item provider.InferenceItem) (string, []block) {
	switch item := item.(type) {
	case *provider.UserMessage:
		return "user", userContent(item.Content)
	case *provider.UserSteer:
		return "user", userContent(item.Content)
	case *provider.AssistantText:
		return "assistant", []block{newTextBlock(item.Text)}
	case *provider.AssistantThinking:
		if item.KeptPastSummary || item.Signature == nil {
			return "", nil
		}
		signature, ok := strings.CutPrefix(*item.Signature, signatureTag)
		if !ok {
			return "", nil
		}
		return "assistant", []block{&thinkingBlock{Type: "thinking", Thinking: item.Text, Signature: signature}}
	case *provider.AssistantRedactedThinking:
		data, ok := strings.CutPrefix(item.Data, signatureTag)
		if item.KeptPastSummary || !ok {
			return "", nil
		}
		return "assistant", []block{&redactedThinkingBlock{Type: "redacted_thinking", Data: data}}
	case *provider.ToolUse:
		input := item.Input
		if len(input) == 0 || strings.TrimSpace(string(input)) == "null" {
			input = json.RawMessage(`{}`)
		}
		return "assistant", []block{
			&toolUseBlock{Type: "tool_use", ID: item.ToolUseID, Name: item.ToolName, Input: input},
		}
	case *provider.ToolResult:
		b := &toolResultBlock{
			Type:      "tool_result",
			ToolUseID: item.ToolUseID,
			Content:   resultContent(item.Output),
			IsError:   item.IsError,
		}
		return "user", []block{b}
	}
	return "", nil
}

// newTextBlock constructs a Messages text block.
func newTextBlock(text string) *textBlock {
	return &textBlock{Type: "text", Text: text}
}

// source constructs the vendor's inline base64 source.
func source(bytes provider.MediaBytes) base64Source {
	return base64Source{Type: "base64", MediaType: bytes.MediaType, Data: base64.StdEncoding.EncodeToString(bytes.Data)}
}

// userContent maps native media and the API's unsupported-media placeholders.
func userContent(parts []provider.UserPart) []block {
	out := make([]block, 0, len(parts))
	for _, part := range parts {
		switch p := part.(type) {
		case *provider.TextPart:
			out = append(out, newTextBlock(p.Text))
		case *provider.VideoPart:
			out = append(out, newTextBlock("[video]"))
		case *provider.ImagePart:
			switch medium := p.Medium.(type) {
			case *provider.MediaBytes:
				out = append(out, &imageBlock{Type: "image", Source: source(*medium)})
			case *provider.MediaURL:
				out = append(out, newTextBlock("[image:"+medium.URL+"]"))
			}
		case *provider.DocumentPart:
			out = append(out, &documentBlock{Type: "document", Source: source(p.Bytes), Title: p.FileName})
		}
	}
	return out
}

// resultContent maps a tool result's text and media onto Messages blocks.
func resultContent(parts []provider.ResultPart) []resultBlock {
	out := make([]resultBlock, 0, len(parts))
	for _, part := range parts {
		switch p := part.(type) {
		case *provider.TextPart:
			out = append(out, newTextBlock(p.Text))
		case *provider.ResultImage:
			out = append(out, &imageBlock{Type: "image", Source: source(p.Bytes)})
		case *provider.ResultVideo:
			out = append(out, newTextBlock("[video:"+p.Bytes.MediaType+"]"))
		}
	}
	return out
}

func markRequestCache(body *requestBody, answered, lastMarkable *content) {
	if len(body.System) > 0 {
		mark(body.System[len(body.System)-1])
	} else if len(body.Tools) > 0 {
		body.Tools[len(body.Tools)-1].CacheControl = &cacheControl{Type: "ephemeral", TTL: "1h"}
	}
	if answered != nil {
		mark(answered)
	}
	if lastMarkable != nil {
		mark(lastMarkable)
	}
}
