package google

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
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
	Name        string          `json:"name"`
	Description string          `json:"description"`
	Parameters  json.RawMessage `json:"parameters"`
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
	b := body{
		Contents:         contents,
		GenerationConfig: generationConfig{MaxOutputTokens: 32000, ThinkingConfig: thinking(request.Thinking)},
	}
	if limit := request.MaxOutputTokens(); limit != nil {
		b.GenerationConfig.MaxOutputTokens = *limit
	}
	if !types.IsBlank(request.SystemPrompt) {
		b.SystemInstruction = &instruction{Parts: []textPart{{Text: request.SystemPrompt}}}
	}
	if len(request.Tools) != 0 {
		declarations := make([]declaration, 0, len(request.Tools))
		for _, tool := range request.Tools {
			schema, err := geminiSchema(tool.InputSchema)
			if err != nil {
				return nil, err
			}
			declarations = append(
				declarations,
				declaration{Name: tool.Name, Description: tool.Description, Parameters: schema},
			)
		}
		b.Tools = []tools{{FunctionDeclarations: declarations}}
	}
	return provider.JSONBody(b)
}

func thinking(config types.ThinkingConfig) thinkingConfig {
	result := thinkingConfig{IncludeThoughts: true}
	var budget uint32
	switch config := config.(type) {
	case nil:
		return result
	case *types.DisabledConfig:
		result.IncludeThoughts = false
	case *types.BudgetConfig:
		budget = config.BudgetTokens
	case *types.EffortConfig:
		budget = provider.EffortBudget(config.Effort)
	case *types.AdaptiveConfig:
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
			var err error
			parts, err = replayCall(item, signature, asText)
			if err != nil {
				return nil, err
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
		contents = appendContent(contents, role, parts)
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
	return append(
		[]any{
			resultPart{
				FunctionResponse: functionResponse{
					Name:     name,
					ID:       result.ToolUseID,
					Response: toolOutput{Output: text},
				},
			},
		},
		media...)
}

// geminiSchema retains Gemini's supported keywords in their read order.
// Values outside schema containers stay raw so their nested object order survives.
func geminiSchema(schema json.RawMessage) (json.RawMessage, error) {
	if !bytes.HasPrefix(bytes.TrimSpace(schema), []byte("{")) {
		if err := contract.CheckJSON(schema); err != nil {
			return nil, err
		}
		return contract.EncodeObject(nil)
	}
	fields, err := contract.ObjectFields(schema)
	if err != nil {
		return nil, err
	}
	kept := make([]contract.Field, 0, len(fields))
	for _, field := range fields {
		// ObjectFields returns each value as a RawMessage, preserving its bytes.
		value := field.Value.(json.RawMessage)
		switch field.Name {
		case "type",
			"format",
			"title",
			"description",
			"nullable",
			"enum",
			"required",
			"minimum",
			"maximum",
			"minItems",
			"maxItems",
			"default":
		case "properties":
			value, err = geminiProperties(value)
			if err != nil {
				return nil, err
			}
		case "items":
			value, err = geminiSchema(value)
			if err != nil {
				return nil, err
			}
		case "anyOf":
			value, err = geminiAlternatives(value)
			if err != nil {
				return nil, err
			}
		default:
			continue
		}
		kept = append(kept, contract.Field{Name: field.Name, Value: value})
	}
	return contract.EncodeObject(kept)
}

func replayCall(item *provider.ToolUse, signature string, asText map[string]bool) ([]any, error) {
	var parts []any
	if signature != "" {
		args := item.Input
		if len(args) == 0 || bytes.Equal(bytes.TrimSpace(args), []byte("null")) {
			args = json.RawMessage(`{}`)
		}
		parts = []any{
			callPart{
				FunctionCall:     requestCall{Name: item.ToolName, Args: args, ID: item.ToolUseID},
				ThoughtSignature: signature,
			},
		}
	} else {
		asText[item.ToolUseID] = true
		args, err := provider.ToolArguments(item.Input)
		if err != nil {
			return nil, err
		}
		parts = []any{textPart{Text: fmt.Sprintf("[called %s with %s]", item.ToolName, args)}}
	}
	return parts, nil
}

func geminiProperties(value json.RawMessage) (json.RawMessage, error) {
	if bytes.HasPrefix(bytes.TrimSpace(value), []byte("{")) {
		properties, err := contract.ObjectFields(value)
		if err != nil {
			return nil, err
		}
		for i := range properties {
			child, err := geminiSchema(properties[i].Value.(json.RawMessage))
			if err != nil {
				return nil, err
			}
			properties[i].Value = child
		}
		value, err = contract.EncodeObject(properties)
		if err != nil {
			return nil, err
		}
	}
	return value, nil
}

func geminiAlternatives(value json.RawMessage) (json.RawMessage, error) {
	if bytes.HasPrefix(bytes.TrimSpace(value), []byte("[")) {
		options, err := contract.Decode[[]json.RawMessage](value)
		if err != nil {
			return nil, err
		}
		for i := range options {
			options[i], err = geminiSchema(options[i])
			if err != nil {
				return nil, err
			}
		}
		value, err = contract.EncodeJSON(options)
		if err != nil {
			return nil, err
		}
	}
	return value, nil
}

func appendContent(contents []content, role string, parts []any) []content {
	if len(parts) == 0 {
		return contents
	}
	if len(contents) > 0 && contents[len(contents)-1].Role == role {
		contents[len(contents)-1].Parts = append(contents[len(contents)-1].Parts, parts...)
	} else {
		contents = append(contents, content{Role: role, Parts: parts})
	}
	return contents
}
