package anthropicapi

import (
	"encoding/json/jsontext"
	"strings"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

const SignatureTag = "anthropic:"

//demi:wire
type requestBody struct {
	Model        string          `json:"model"`
	Messages     []message       `json:"messages"`
	MaxTokens    uint32          `json:"max_tokens"`
	Stream       bool            `json:"stream"`
	System       *[]contentBlock `json:"system,omitzero"`
	Tools        *[]tool         `json:"tools,omitzero"`
	Thinking     *thinking       `json:"thinking,omitzero"`
	OutputConfig *outputConfig   `json:"output_config,omitzero"`
	ServiceTier  *string         `json:"service_tier,omitzero"`
}

//demi:wire
type message struct {
	Role    string         `json:"role" check:"oneof=user|assistant"`
	Content []contentBlock `json:"content"`
}

//demi:wire
type contentBlock struct {
	Type         string          `json:"type" check:"oneof=text|image|document|tool_use|tool_result|thinking|redacted_thinking"`
	Text         *string         `json:"text,omitzero"`
	Source       *base64Source   `json:"source,omitzero"`
	Title        *string         `json:"title,omitzero"`
	ID           *string         `json:"id,omitzero"`
	Name         *string         `json:"name,omitzero"`
	Input        *jsontext.Value `json:"input,omitzero"`
	ToolUseID    *string         `json:"tool_use_id,omitzero"`
	Content      *[]resultBlock  `json:"content,omitzero"`
	IsError      *bool           `json:"is_error,omitzero"`
	Thinking     *string         `json:"thinking,omitzero"`
	Signature    *string         `json:"signature,omitzero"`
	Data         *string         `json:"data,omitzero"`
	CacheControl *cacheControl   `json:"cache_control,omitzero"`
}

//demi:wire
type resultBlock struct {
	Type   string        `json:"type" check:"oneof=text|image"`
	Text   *string       `json:"text,omitzero"`
	Source *base64Source `json:"source,omitzero"`
}

//demi:wire
type base64Source struct {
	Type      string        `json:"type" check:"eq=base64"`
	MediaType string        `json:"media_type"`
	Data      core.B64Bytes `json:"data" check:"func=core.Validate"`
}

//demi:wire
type cacheControl struct {
	Type string `json:"type" check:"eq=ephemeral"`
	TTL  string `json:"ttl" check:"eq=1h"`
}

//demi:wire
type tool struct {
	Name         string         `json:"name"`
	Description  string         `json:"description"`
	InputSchema  jsontext.Value `json:"input_schema"`
	CacheControl *cacheControl  `json:"cache_control,omitzero"`
}

//demi:wire
type thinking struct {
	Type         string  `json:"type" check:"oneof=enabled|adaptive"`
	BudgetTokens *uint32 `json:"budget_tokens,omitzero"`
	Display      *string `json:"display,omitzero"`
}

//demi:wire
type outputConfig struct {
	Effort string `json:"effort"`
}

func body(request provider.InferenceRequest) []byte {
	maxTokens := uint32(32000)
	if limit := request.MaxOutputTokens(); limit != nil {
		maxTokens = *limit
	}
	result := requestBody{Model: request.ModelID, Messages: []message{}, MaxTokens: maxTokens, Stream: true, ServiceTier: request.ServiceTierID}
	if !core.IsBlank(request.SystemPrompt) {
		system := []contentBlock{{Type: "text", Text: &request.SystemPrompt}}
		result.System = &system
	}
	if len(request.Tools) > 0 {
		tools := make([]tool, 0, len(request.Tools))
		for _, definition := range request.Tools {
			tools = append(tools, tool{Name: definition.Name, Description: definition.Description, InputSchema: definition.InputSchema})
		}
		result.Tools = &tools
	}
	switch config := request.Thinking.(type) {
	case core.ThinkingConfigBudget:
		ceiling := max(uint32(1024), maxTokens-min(maxTokens, 1024))
		budget := max(uint32(1024), min(config.BudgetTokens, ceiling))
		result.Thinking = &thinking{Type: "enabled", BudgetTokens: &budget}
	case core.ThinkingConfigAdaptive:
		display := "summarized"
		result.Thinking = &thinking{Type: "adaptive", Display: &display}
		result.OutputConfig = &outputConfig{Effort: config.Effort}
	case core.ThinkingConfigEffort:
		display := "summarized"
		if config.Summary != nil && *config.Summary == core.ThinkingSummaryOff {
			display = "omitted"
		}
		result.Thinking = &thinking{Type: "adaptive", Display: &display}
		result.OutputConfig = &outputConfig{Effort: config.Effort}
	}
	type position struct{ message, block int }
	last := func() *position {
		if len(result.Messages) == 0 {
			return nil
		}
		m := len(result.Messages) - 1
		return &position{message: m, block: len(result.Messages[m].Content) - 1}
	}
	var answered *position
	for index, item := range request.Items {
		role, blocks := itemContent(item)
		if len(blocks) > 0 {
			end := len(result.Messages) - 1
			if end >= 0 && result.Messages[end].Role == role {
				result.Messages[end].Content = append(result.Messages[end].Content, blocks...)
			} else {
				result.Messages = append(result.Messages, message{Role: role, Content: blocks})
			}
		}
		if request.PromptCache != nil && index+1 == request.PromptCache.AnsweredItems {
			answered = last()
		}
	}
	if request.PromptCache != nil {
		mark := &cacheControl{Type: "ephemeral", TTL: "1h"}
		if result.System != nil {
			(*result.System)[len(*result.System)-1].CacheControl = mark
		} else if result.Tools != nil {
			(*result.Tools)[len(*result.Tools)-1].CacheControl = mark
		}
		for _, position := range []*position{answered, last()} {
			if position == nil {
				continue
			}
			found := false
			for m := position.message; m >= 0 && !found; m-- {
				end := len(result.Messages[m].Content) - 1
				if m == position.message {
					end = position.block
				}
				for b := end; b >= 0; b-- {
					block := &result.Messages[m].Content[b]
					if block.Type != "thinking" && block.Type != "redacted_thinking" {
						block.CacheControl = mark
						found = true
						break
					}
				}
			}
		}
	}
	return provider.JSONBody(result)
}
func source(bytes provider.MediaBytes) *base64Source {
	return &base64Source{Type: "base64", MediaType: bytes.MediaType, Data: bytes.Data}
}
func userContent(content []provider.UserPart) []contentBlock {
	blocks := make([]contentBlock, 0, len(content))
	for _, part := range content {
		switch part := part.(type) {
		case provider.TextPart:
			blocks = append(blocks, contentBlock{Type: "text", Text: &part.Text})
		case provider.VideoPart:
			text := "[video]"
			blocks = append(blocks, contentBlock{Type: "text", Text: &text})
		case provider.ImagePart:
			switch medium := part.Medium.(type) {
			case provider.MediaBytes:
				blocks = append(blocks, contentBlock{Type: "image", Source: source(medium)})
			case provider.MediaURL:
				text := "[image:" + medium.URL + "]"
				blocks = append(blocks, contentBlock{Type: "text", Text: &text})
			}
		case provider.DocumentPart:
			blocks = append(blocks, contentBlock{Type: "document", Source: source(part.Bytes), Title: &part.FileName})
		}
	}
	return blocks
}
func itemContent(item provider.InferenceItem) (string, []contentBlock) {
	switch item := item.(type) {
	case provider.UserMessage:
		return "user", userContent(item.Content)
	case provider.UserSteer:
		return "user", userContent(item.Content)
	case provider.AssistantText:
		return "assistant", []contentBlock{{Type: "text", Text: &item.Text}}
	case provider.AssistantThinking:
		if item.KeptPastSummary || item.Signature == nil || !strings.HasPrefix(*item.Signature, SignatureTag) {
			return "", nil
		}
		signature := strings.TrimPrefix(*item.Signature, SignatureTag)
		return "assistant", []contentBlock{{Type: "thinking", Thinking: &item.Text, Signature: &signature}}
	case provider.AssistantRedactedThinking:
		if item.KeptPastSummary || !strings.HasPrefix(item.Data, SignatureTag) {
			return "", nil
		}
		data := strings.TrimPrefix(item.Data, SignatureTag)
		return "assistant", []contentBlock{{Type: "redacted_thinking", Data: &data}}
	case provider.ToolUse:
		input := item.Input
		if len(input) == 0 || string(input) == "null" {
			input = jsontext.Value(`{}`)
		}
		return "assistant", []contentBlock{{Type: "tool_use", ID: &item.ToolUseID, Name: &item.ToolName, Input: &input}}
	case provider.ToolResult:
		results := make([]resultBlock, 0, len(item.Output))
		for _, part := range item.Output {
			switch part := part.(type) {
			case provider.TextPart:
				results = append(results, resultBlock{Type: "text", Text: &part.Text})
			case provider.ResultImage:
				results = append(results, resultBlock{Type: "image", Source: source(part.Bytes)})
			case provider.ResultVideo:
				text := "[video:" + part.Bytes.MediaType + "]"
				results = append(results, resultBlock{Type: "text", Text: &text})
			}
		}
		block := contentBlock{Type: "tool_result", ToolUseID: &item.ToolUseID, Content: &results}
		if item.IsError {
			block.IsError = &item.IsError
		}
		return "user", []contentBlock{block}
	}
	return "", nil
}
