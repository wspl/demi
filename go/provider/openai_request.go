package provider

import (
	"encoding/base64"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"strings"

	"github.com/wspl/demi/go/core"
)

// JSONBody serializes a provider's already validated request values.
func JSONBody(value any) jsontext.Value {
	body, err := json.Marshal(value, json.Deterministic(true))
	if err != nil {
		panic(fmt.Sprintf("provider request body: %v", err))
	}
	return body
}
func ReasoningEffort(thinking core.ThinkingConfig) *string {
	switch t := thinking.(type) {
	case core.ThinkingConfigAdaptive:
		return &t.Effort
	case core.ThinkingConfigEffort:
		return &t.Effort
	}
	return nil
}

//demi:wire
type ResponsesReasoningConfig struct {
	Effort  string  `json:"effort"`
	Summary *string `json:"summary,omitzero"`
}

func ResponsesReasoning(thinking core.ThinkingConfig, offAsAuto bool) *ResponsesReasoningConfig {
	effort := ReasoningEffort(thinking)
	if effort == nil {
		return nil
	}
	summary := string(core.ThinkingSummaryAuto)
	result := &ResponsesReasoningConfig{Effort: *effort, Summary: &summary}
	if t, ok := thinking.(core.ThinkingConfigEffort); ok {
		if t.Effort == "none" {
			result.Summary = nil
			return result
		}
		if t.Summary != nil {
			if *t.Summary == core.ThinkingSummaryOff {
				if !offAsAuto {
					result.Summary = nil
				}
			} else {
				summary = string(*t.Summary)
			}
		}
	}
	return result
}

type AssistantReplay uint8

const (
	AssistantMinimal AssistantReplay = iota
	AssistantCompleted
	AssistantIdentified
)

type ReasoningReplay uint8

const (
	ReasoningReplayable ReasoningReplay = iota
	ReasoningWhole
)

type ToolMedia uint8

const (
	ToolMediaFollowUp ToolMedia = iota
	ToolMediaInline
)

type ResponsesDialect struct {
	SignatureTag string
	Assistant    AssistantReplay
	Reasoning    ReasoningReplay
	ToolMedia    ToolMedia
}

//demi:wire
type ResponsesTool struct {
	Type        string          `json:"type" check:"eq=function"`
	Name        string          `json:"name"`
	Description string          `json:"description"`
	Parameters  jsontext.Value  `json:"parameters"`
	Strict      *jsontext.Value `json:"strict,omitzero"`
}

func NewResponsesTool(tool ToolDefinition, strictNull bool) ResponsesTool {
	result := ResponsesTool{Type: "function", Name: tool.Name, Description: tool.Description, Parameters: tool.InputSchema}
	if strictNull {
		value := jsontext.Value("null")
		result.Strict = &value
	}
	return result
}

//demi:wire
type ResponsesUserInput struct {
	Role    string           `json:"role" check:"eq=user"`
	Content []jsontext.Value `json:"content"`
}

//demi:wire
type ResponsesAssistantInput struct {
	Type    string                `json:"type" check:"eq=message"`
	Role    string                `json:"role" check:"eq=assistant"`
	ID      *string               `json:"id,omitzero"`
	Status  *string               `json:"status,omitzero"`
	Content []ResponsesOutputPart `json:"content"`
}

//demi:wire
type ResponsesOutputPart struct {
	Type        string           `json:"type" check:"eq=output_text"`
	Text        string           `json:"text"`
	Annotations []jsontext.Value `json:"annotations"`
}

//demi:wire
type ResponsesFunctionInput struct {
	Type      string  `json:"type" check:"eq=function_call"`
	ID        *string `json:"id,omitzero"`
	CallID    string  `json:"call_id"`
	Name      string  `json:"name"`
	Arguments string  `json:"arguments"`
}

//demi:wire
type ResponsesFunctionOutput struct {
	Type   string         `json:"type" check:"eq=function_call_output"`
	CallID string         `json:"call_id"`
	Output jsontext.Value `json:"output"`
}

//demi:wire
type ResponsesInputText struct {
	Type string `json:"type" check:"eq=input_text"`
	Text string `json:"text"`
}

//demi:wire
type ResponsesInputImage struct {
	Type     string `json:"type" check:"eq=input_image"`
	ImageURL string `json:"image_url"`
	Detail   string `json:"detail" check:"eq=auto"`
}

//demi:wire
type ResponsesInputFile struct {
	Type     string `json:"type" check:"eq=input_file"`
	Filename string `json:"filename"`
	FileData string `json:"file_data"`
}

func ResponsesInput(items []InferenceItem, dialect ResponsesDialect) []jsontext.Value {
	input := make([]jsontext.Value, 0, len(items))
	for index, item := range items {
		switch item := item.(type) {
		case UserMessage:
			input = append(input, JSONBody(ResponsesUserInput{Role: "user", Content: responsesUserParts(item.Content)}))
		case UserSteer:
			input = append(input, JSONBody(ResponsesUserInput{Role: "user", Content: responsesUserParts(item.Content)}))
		case AssistantText:
			message := ResponsesAssistantInput{Type: "message", Role: "assistant", Content: []ResponsesOutputPart{{Type: "output_text", Text: item.Text, Annotations: []jsontext.Value{}}}}
			if dialect.Assistant != AssistantMinimal {
				value := "completed"
				message.Status = &value
			}
			if dialect.Assistant == AssistantIdentified {
				value := "msg_" + ShortHash(fmt.Sprintf("%d:%s:%s", index, item.ModelID, item.Text))
				message.ID = &value
			}
			input = append(input, JSONBody(message))
		case AssistantThinking:
			if item.Signature == nil || !strings.HasPrefix(*item.Signature, dialect.SignatureTag) {
				continue
			}
			reasoning, err := decode[ReasoningItem]([]byte(strings.TrimPrefix(*item.Signature, dialect.SignatureTag)))
			if err != nil {
				continue
			} // A signature of another shape cannot be replayed.
			if dialect.Reasoning == ReasoningReplayable {
				reasoning.Extra = nil
			}
			input = append(input, JSONBody(reasoning))
		case ToolUse:
			callID, itemID := SplitToolUseID(item.ToolUseID)
			input = append(input, JSONBody(ResponsesFunctionInput{Type: "function_call", ID: itemID, CallID: callID, Name: item.ToolName, Arguments: ToolArguments(item.Input)}))
		case ToolResult:
			callID, _ := SplitToolUseID(item.ToolUseID)
			if dialect.ToolMedia == ToolMediaFollowUp {
				input = append(input, JSONBody(ResponsesFunctionOutput{Type: "function_call_output", CallID: callID, Output: JSONBody(ToolOutputText(item.Output))}))
				parts := []jsontext.Value{}
				for _, part := range item.Output {
					if bytes := resultMedia(part); bytes != nil {
						parts = append(parts, JSONBody(ResponsesInputImage{Type: "input_image", ImageURL: DataURL(*bytes), Detail: "auto"}))
					}
				}
				if len(parts) > 0 {
					parts = append([]jsontext.Value{JSONBody(ResponsesInputText{Type: "input_text", Text: fmt.Sprintf("[media returned by tool call %s]", callID)})}, parts...)
					input = append(input, JSONBody(ResponsesUserInput{Role: "user", Content: parts}))
				}
			} else {
				texts := []string{}
				images := []jsontext.Value{}
				for _, part := range item.Output {
					switch part := part.(type) {
					case TextPart:
						texts = append(texts, part.Text)
					case ResultImage:
						images = append(images, JSONBody(ResponsesInputImage{Type: "input_image", ImageURL: DataURL(part.Bytes), Detail: "auto"}))
					}
				}
				text := strings.Join(texts, "\n")
				output := JSONBody(text)
				if len(images) > 0 {
					if text != "" {
						images = append([]jsontext.Value{JSONBody(ResponsesInputText{Type: "input_text", Text: text})}, images...)
					}
					output = JSONBody(images)
				}
				input = append(input, JSONBody(ResponsesFunctionOutput{Type: "function_call_output", CallID: callID, Output: output}))
			}
		}
	}
	return input
}
func responsesUserParts(content []UserPart) []jsontext.Value {
	parts := make([]jsontext.Value, 0, len(content))
	for _, part := range content {
		switch part := part.(type) {
		case TextPart:
			parts = append(parts, JSONBody(ResponsesInputText{Type: "input_text", Text: part.Text}))
		case DocumentPart:
			parts = append(parts, JSONBody(ResponsesInputFile{Type: "input_file", Filename: part.FileName, FileData: DataURL(part.Bytes)}))
		case ImagePart:
			parts = append(parts, JSONBody(ResponsesInputImage{Type: "input_image", ImageURL: MediaURLText(part.Medium), Detail: "auto"}))
		case VideoPart:
			parts = append(parts, JSONBody(ResponsesInputImage{Type: "input_image", ImageURL: MediaURLText(part.Medium), Detail: "auto"}))
		}
	}
	return parts
}
func DataURL(bytes MediaBytes) string {
	return "data:" + bytes.MediaType + ";base64," + base64.StdEncoding.EncodeToString(bytes.Data.Bytes())
}
func MediaURLText(medium Medium) string {
	switch medium := medium.(type) {
	case MediaBytes:
		return DataURL(medium)
	case MediaURL:
		return medium.URL
	}
	panic("invalid provider medium")
}
func resultMedia(part ResultPart) *MediaBytes {
	switch part := part.(type) {
	case ResultImage:
		return &part.Bytes
	case ResultVideo:
		return &part.Bytes
	}
	return nil
}
func ToolOutputText(output []ResultPart) string {
	texts := make([]string, 0, len(output))
	for _, part := range output {
		switch part := part.(type) {
		case TextPart:
			texts = append(texts, part.Text)
		case ResultImage:
			texts = append(texts, "[image:"+part.Bytes.MediaType+"]")
		case ResultVideo:
			texts = append(texts, "[video:"+part.Bytes.MediaType+"]")
		}
	}
	return strings.Join(texts, "\n")
}
func ToolArguments(input jsontext.Value) string {
	if len(input) == 0 || string(input) == "null" {
		return "{}"
	}
	if input.Kind() == '"' {
		var text string
		if err := json.Unmarshal(input, &text); err != nil {
			panic(err)
		}
		return text
	}
	compact := input.Clone()
	if err := compact.Compact(); err != nil {
		panic(err)
	}
	return string(compact)
}
