package google

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"slices"
	"strings"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

//demi:wire
type requestBody struct {
	Contents          []content          `json:"contents"`
	SystemInstruction *systemInstruction `json:"systemInstruction,omitzero"`
	Tools             *[]tools           `json:"tools,omitzero"`
	GenerationConfig  generationConfig   `json:"generationConfig"`
}

//demi:wire
type content struct {
	Role  string `json:"role" check:"oneof=user|model"`
	Parts []part `json:"parts"`
}

//demi:wire
type systemInstruction struct {
	Parts []textPart `json:"parts"`
}

//demi:wire
type textPart struct {
	Text string `json:"text"`
}

//demi:wire
type tools struct {
	FunctionDeclarations []functionDeclaration `json:"functionDeclarations"`
}

//demi:wire
type functionDeclaration struct {
	Name        string         `json:"name"`
	Description string         `json:"description"`
	Parameters  jsontext.Value `json:"parameters"`
}

//demi:wire
type generationConfig struct {
	MaxOutputTokens uint32         `json:"maxOutputTokens"`
	ThinkingConfig  geminiThinking `json:"thinkingConfig"`
}

//demi:wire
type geminiThinking struct {
	IncludeThoughts bool    `json:"includeThoughts"`
	ThinkingBudget  *uint32 `json:"thinkingBudget,omitzero"`
}

//demi:wire
type part struct {
	Text             *string           `json:"text,omitzero"`
	InlineData       *blob             `json:"inlineData,omitzero"`
	FileData         *fileURI          `json:"fileData,omitzero"`
	FunctionCall     *functionCall     `json:"functionCall,omitzero"`
	ThoughtSignature *string           `json:"thoughtSignature,omitzero"`
	FunctionResponse *functionResponse `json:"functionResponse,omitzero"`
}

//demi:wire
type blob struct {
	MIMEType string        `json:"mimeType"`
	Data     core.B64Bytes `json:"data" check:"func=core.Validate"`
}

//demi:wire
type fileURI struct {
	FileURI string `json:"fileUri"`
}

//demi:wire
type functionCall struct {
	Name string         `json:"name"`
	Args jsontext.Value `json:"args"`
	ID   string         `json:"id"`
}

//demi:wire
type functionResponse struct {
	Name     string     `json:"name"`
	ID       string     `json:"id"`
	Response toolOutput `json:"response"`
}

//demi:wire
type toolOutput struct {
	Output string `json:"output"`
}

func body(request provider.InferenceRequest) []byte {
	result := requestBody{Contents: contents(request.Items), GenerationConfig: generationConfig{MaxOutputTokens: 32000, ThinkingConfig: geminiThinking{IncludeThoughts: true}}}
	if limit := request.MaxOutputTokens(); limit != nil {
		result.GenerationConfig.MaxOutputTokens = *limit
	}
	switch config := request.Thinking.(type) {
	case core.ThinkingConfigDisabled:
		result.GenerationConfig.ThinkingConfig = geminiThinking{ThinkingBudget: new(uint32(0))}
	case core.ThinkingConfigBudget:
		result.GenerationConfig.ThinkingConfig.ThinkingBudget = &config.BudgetTokens
	case core.ThinkingConfigEffort:
		result.GenerationConfig.ThinkingConfig.ThinkingBudget = effortBudget(config.Effort)
	case core.ThinkingConfigAdaptive:
		result.GenerationConfig.ThinkingConfig.ThinkingBudget = effortBudget(config.Effort)
	}
	if !core.IsBlank(request.SystemPrompt) {
		result.SystemInstruction = &systemInstruction{Parts: []textPart{{Text: request.SystemPrompt}}}
	}
	if len(request.Tools) > 0 {
		declarations := make([]functionDeclaration, 0, len(request.Tools))
		for _, tool := range request.Tools {
			declarations = append(declarations, functionDeclaration{Name: tool.Name, Description: tool.Description, Parameters: geminiSchema(tool.InputSchema)})
		}
		value := []tools{{FunctionDeclarations: declarations}}
		result.Tools = &value
	}
	return provider.JSONBody(result)
}
func effortBudget(effort string) *uint32 {
	budget := uint32(16384)
	switch effort {
	case "low":
		budget = 4096
	case "high":
		budget = 32768
	case "xhigh":
		budget = 65536
	case "max":
		budget = 98304
	}
	return &budget
}
func inline(bytes provider.MediaBytes) part {
	return part{InlineData: &blob{MIMEType: bytes.MediaType, Data: bytes.Data}}
}
func userParts(parts []provider.UserPart) []part {
	result := make([]part, 0, len(parts))
	medium := func(media provider.Medium) part {
		switch media := media.(type) {
		case provider.MediaBytes:
			return inline(media)
		case provider.MediaURL:
			return part{FileData: &fileURI{FileURI: media.URL}}
		}
		panic("invalid provider medium")
	}
	for _, value := range parts {
		switch value := value.(type) {
		case provider.TextPart:
			result = append(result, part{Text: &value.Text})
		case provider.DocumentPart:
			result = append(result, inline(value.Bytes))
		case provider.ImagePart:
			result = append(result, medium(value.Medium))
		case provider.VideoPart:
			result = append(result, medium(value.Medium))
		}
	}
	return result
}
func contents(items []provider.InferenceItem) []content {
	result := []content{}
	names := map[string]string{}
	asText := map[string]bool{}
	var pending *string
	for _, item := range items {
		role := "model"
		var parts []part
		switch item := item.(type) {
		case provider.UserMessage:
			pending = nil
			role = "user"
			parts = userParts(item.Content)
		case provider.UserSteer:
			pending = nil
			role = "user"
			parts = userParts(item.Content)
		case provider.AssistantText:
			pending = nil
			parts = []part{{Text: &item.Text}}
		case provider.AssistantThinking:
			pending = nil
			if item.Signature != nil && strings.HasPrefix(*item.Signature, SignatureTag) {
				value := strings.TrimPrefix(*item.Signature, SignatureTag)
				if value != "" {
					pending = &value
				}
			}
			continue
		case provider.AssistantRedactedThinking:
			continue
		case provider.ToolUse:
			names[item.ToolUseID] = item.ToolName
			if pending != nil {
				input := item.Input
				if len(input) == 0 || string(input) == "null" {
					input = jsontext.Value(`{}`)
				}
				parts = []part{{FunctionCall: &functionCall{Name: item.ToolName, Args: input, ID: item.ToolUseID}, ThoughtSignature: pending}}
			} else {
				asText[item.ToolUseID] = true
				text := fmt.Sprintf("[called %s with %s]", item.ToolName, provider.ToolArguments(item.Input))
				parts = []part{{Text: &text}}
			}
			pending = nil
		case provider.ToolResult:
			pending = nil
			role = "user"
			name, ok := names[item.ToolUseID]
			if !ok {
				name = "tool"
			}
			texts := []string{}
			media := []part{}
			for _, value := range item.Output {
				switch value := value.(type) {
				case provider.TextPart:
					texts = append(texts, value.Text)
				case provider.ResultImage:
					if asText[item.ToolUseID] {
						texts = append(texts, "["+value.Bytes.MediaType+"]")
					} else {
						media = append(media, inline(value.Bytes))
					}
				case provider.ResultVideo:
					if asText[item.ToolUseID] {
						texts = append(texts, "["+value.Bytes.MediaType+"]")
					} else {
						media = append(media, inline(value.Bytes))
					}
				}
			}
			text := strings.Join(texts, "\n")
			if asText[item.ToolUseID] {
				text = "[" + name + " returned] " + text
				parts = []part{{Text: &text}}
			} else {
				parts = append([]part{{FunctionResponse: &functionResponse{Name: name, ID: item.ToolUseID, Response: toolOutput{Output: text}}}}, media...)
			}
		}
		if len(parts) == 0 {
			continue
		}
		end := len(result) - 1
		if end >= 0 && result[end].Role == role {
			result[end].Parts = append(result[end].Parts, parts...)
		} else {
			result = append(result, content{Role: role, Parts: parts})
		}
	}
	return result
}

// geminiSchema drops unsupported JSON Schema keywords in schema containers.
func geminiSchema(raw jsontext.Value) jsontext.Value {
	if raw.Kind() != '{' {
		return jsontext.Value(`{}`)
	}
	var object map[string]jsontext.Value
	if err := json.Unmarshal(raw, &object); err != nil {
		panic(err)
	} // Tool schemas are validated before inference.
	for key, value := range object {
		if !slices.Contains([]string{"type", "format", "title", "description", "nullable", "enum", "items", "properties", "required", "minimum", "maximum", "minItems", "maxItems", "anyOf", "default"}, key) {
			delete(object, key)
			continue
		}
		switch key {
		case "items":
			object[key] = geminiSchema(value)
		case "properties":
			if value.Kind() == '{' {
				var children map[string]jsontext.Value
				if err := json.Unmarshal(value, &children); err != nil {
					panic(err)
				}
				for name, child := range children {
					children[name] = geminiSchema(child)
				}
				object[key] = provider.JSONBody(children)
			}
		case "anyOf":
			if value.Kind() == '[' {
				var children []jsontext.Value
				if err := json.Unmarshal(value, &children); err != nil {
					panic(err)
				}
				for i, child := range children {
					children[i] = geminiSchema(child)
				}
				object[key] = provider.JSONBody(children)
			}
		}
	}
	return provider.JSONBody(object)
}
