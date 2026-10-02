package google

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

const signatureTag = "google:"

type content struct {
	Role  string `json:"role"`
	Parts []any  `json:"parts"`
}
type textPart struct {
	Text string `json:"text"`
}
type inlinePart struct {
	InlineData blob `json:"inlineData"`
}
type blob struct {
	MIMEType string `json:"mimeType"`
	Data     string `json:"data"`
}
type filePart struct {
	FileData fileURI `json:"fileData"`
}
type fileURI struct {
	URI string `json:"fileUri"`
}
type callPart struct {
	FunctionCall     requestCall `json:"functionCall"`
	ThoughtSignature string      `json:"thoughtSignature"`
}
type requestCall struct {
	Name string          `json:"name"`
	Args json.RawMessage `json:"args"`
	ID   string          `json:"id"`
}
type resultPart struct {
	FunctionResponse functionResponse `json:"functionResponse"`
}
type functionResponse struct {
	Name     string     `json:"name"`
	ID       string     `json:"id"`
	Response toolOutput `json:"response"`
}
type toolOutput struct {
	Output string `json:"output"`
}
type thinkingConfig struct {
	IncludeThoughts bool    `json:"includeThoughts"`
	ThinkingBudget  *uint32 `json:"thinkingBudget,omitempty"`
}
type generationConfig struct {
	MaxOutputTokens uint32         `json:"maxOutputTokens"`
	ThinkingConfig  thinkingConfig `json:"thinkingConfig"`
}
type declaration struct {
	Name        string `json:"name"`
	Description string `json:"description"`
	Parameters  any    `json:"parameters"`
}
type tools struct {
	FunctionDeclarations []declaration `json:"functionDeclarations"`
}
type instruction struct {
	Parts []textPart `json:"parts"`
}
type body struct {
	Contents          []content        `json:"contents"`
	SystemInstruction *instruction     `json:"systemInstruction,omitempty"`
	Tools             []tools          `json:"tools,omitempty"`
	GenerationConfig  generationConfig `json:"generationConfig"`
}

func encode(request provider.InferenceRequest) ([]byte, error) {
	contents, err := replay(request.Items)
	if err != nil {
		return nil, err
	}
	b := body{Contents: contents, GenerationConfig: generationConfig{MaxOutputTokens: 32000, ThinkingConfig: thinking(request.Thinking)}}
	if limit := request.MaxOutputTokens(); limit != nil {
		b.GenerationConfig.MaxOutputTokens = *limit
	}
	if !core.IsBlank(request.SystemPrompt) {
		b.SystemInstruction = &instruction{Parts: []textPart{{Text: request.SystemPrompt}}}
	}
	if len(request.Tools) != 0 {
		declarations := make([]declaration, 0, len(request.Tools))
		for _, tool := range request.Tools {
			schema, err := provider.DecodeUntagged[any](string(tool.InputSchema))
			if err != nil {
				return nil, err
			}
			declarations = append(declarations, declaration{Name: tool.Name, Description: tool.Description, Parameters: geminiSchema(schema)})
		}
		b.Tools = []tools{{FunctionDeclarations: declarations}}
	}
	return provider.JSONBody(b)
}
func thinking(config core.ThinkingConfig) thinkingConfig {
	result := thinkingConfig{IncludeThoughts: true}
	var budget uint32
	switch config := config.(type) {
	case nil:
		return result
	case *core.DisabledConfig:
		result.IncludeThoughts = false
	case *core.BudgetConfig:
		budget = config.BudgetTokens
	case *core.EffortConfig:
		budget = provider.EffortBudget(config.Effort)
	case *core.AdaptiveConfig:
		budget = provider.EffortBudget(config.Effort)
	}
	result.ThinkingBudget = &budget
	return result
}

// replay pairs Gemini calls with their preceding signatures and merges adjacent roles.
func replay(items []provider.InferenceItem) ([]content, error) {
	contents := make([]content, 0)
	names := make(map[string]string)
	asText := make(map[string]bool)
	signature := ""
	for _, item := range items {
		role := "model"
		var parts []any
		switch item := item.(type) {
		case *provider.UserMessage:
			signature = ""
			role, parts = "user", userParts(item.Content)
		case *provider.UserSteer:
			signature = ""
			role, parts = "user", userParts(item.Content)
		case *provider.AssistantText:
			signature = ""
			parts = []any{textPart{Text: item.Text}}
		case *provider.AssistantThinking:
			signature = ""
			if item.Signature != nil && strings.HasPrefix(*item.Signature, signatureTag) {
				signature = strings.TrimPrefix(*item.Signature, signatureTag)
			}
			continue
		case *provider.AssistantRedactedThinking:
			continue
		case *provider.ToolUse:
			names[item.ToolUseID] = item.ToolName
			if signature != "" {
				args := item.Input
				if len(args) == 0 || bytes.Equal(bytes.TrimSpace(args), []byte("null")) {
					args = json.RawMessage(`{}`)
				}
				parts = []any{callPart{FunctionCall: requestCall{Name: item.ToolName, Args: args, ID: item.ToolUseID}, ThoughtSignature: signature}}
			} else {
				asText[item.ToolUseID] = true
				args, err := provider.ToolArguments(item.Input)
				if err != nil {
					return nil, err
				}
				parts = []any{textPart{Text: fmt.Sprintf("[called %s with %s]", item.ToolName, args)}}
			}
			signature = ""
		case *provider.ToolResult:
			signature = ""
			role = "user"
			name, ok := names[item.ToolUseID]
			if !ok {
				name = "tool"
			}
			parts = resultParts(item, name, asText[item.ToolUseID])
		}
		if len(parts) == 0 {
			continue
		}
		if len(contents) > 0 && contents[len(contents)-1].Role == role {
			contents[len(contents)-1].Parts = append(contents[len(contents)-1].Parts, parts...)
		} else {
			contents = append(contents, content{Role: role, Parts: parts})
		}
	}
	return contents, nil
}

func userParts(parts []provider.UserPart) []any {
	out := make([]any, 0, len(parts))
	for _, part := range parts {
		switch part := part.(type) {
		case *provider.TextPart:
			out = append(out, textPart{Text: part.Text})
		case *provider.ImagePart:
			out = append(out, mediumPart(part.Medium))
		case *provider.VideoPart:
			out = append(out, mediumPart(part.Medium))
		case *provider.DocumentPart:
			out = append(out, inline(part.Bytes))
		}
	}
	return out
}
func mediumPart(medium provider.Medium) any {
	switch medium := medium.(type) {
	case *provider.MediaBytes:
		return inline(*medium)
	case *provider.MediaURL:
		return filePart{FileData: fileURI{URI: medium.URL}}
	}
	return nil
}
func inline(media provider.MediaBytes) any {
	return inlinePart{InlineData: blob{MIMEType: media.MediaType, Data: base64.StdEncoding.EncodeToString(media.Data)}}
}

// resultParts keeps Gemini's JSON result separate from its inline media.
func resultParts(result *provider.ToolResult, name string, asText bool) []any {
	texts := make([]string, 0)
	media := make([]any, 0)
	for _, part := range result.Output {
		var data *provider.MediaBytes
		switch part := part.(type) {
		case *provider.TextPart:
			texts = append(texts, part.Text)
		case *provider.ResultImage:
			data = &part.Bytes
		case *provider.ResultVideo:
			data = &part.Bytes
		}
		if data != nil {
			if asText {
				texts = append(texts, "["+data.MediaType+"]")
			} else {
				media = append(media, inline(*data))
			}
		}
	}
	text := strings.Join(texts, "\n")
	if asText {
		return []any{textPart{Text: "[" + name + " returned] " + text}}
	}
	return append([]any{resultPart{FunctionResponse: functionResponse{Name: name, ID: result.ToolUseID, Response: toolOutput{Output: text}}}}, media...)
}

// geminiSchema retains only Gemini's supported schema keywords, including nested schemas.
func geminiSchema(schema any) any {
	kept := make(map[string]any)
	object, ok := schema.(map[string]any)
	if !ok {
		return kept
	}
	for key, value := range object {
		switch key {
		case "type", "format", "title", "description", "nullable", "enum", "required", "minimum", "maximum", "minItems", "maxItems", "default":
		case "properties":
			if properties, ok := value.(map[string]any); ok {
				children := make(map[string]any, len(properties))
				for name, child := range properties {
					children[name] = geminiSchema(child)
				}
				value = children
			}
		case "items":
			value = geminiSchema(value)
		case "anyOf":
			if options, ok := value.([]any); ok {
				children := make([]any, len(options))
				for i, child := range options {
					children[i] = geminiSchema(child)
				}
				value = children
			}
		default:
			continue
		}
		kept[key] = value
	}
	return kept
}
