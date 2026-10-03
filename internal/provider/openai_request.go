package provider

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
	"unicode/utf16"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
)

// ReasoningEffort reads an effort-based thinking configuration.
func ReasoningEffort(thinking core.ThinkingConfig) *string {
	switch config := thinking.(type) {
	case *core.AdaptiveConfig:
		return &config.Effort
	case *core.EffortConfig:
		return &config.Effort
	case *core.BudgetConfig, *core.DisabledConfig:
		return nil
	}
	return nil
}

// PromptCacheKey limits a session ID to 64 UTF-16 units by hashing longer IDs.
func PromptCacheKey(sessionID string) string {
	if len(utf16.Encode([]rune(sessionID))) <= 64 {
		return sessionID
	}
	return "session_" + ShortHash(sessionID)
}

// ShortHash is hexadecimal FNV-1a over UTF-16 code units, without leading zeros.
func ShortHash(text string) string {
	hash := uint32(2166136261)
	for _, unit := range utf16.Encode([]rune(text)) {
		hash ^= uint32(unit)
		hash *= 16777619
	}
	return strconv.FormatUint(uint64(hash), 16)
}

// Reasoning is the reasoning object of a Responses request.
type Reasoning struct {
	Effort  string                `json:"effort"`
	Summary *core.ThinkingSummary `json:"summary,omitempty"`
}

// SummaryOff specifies how an endpoint accepts a disabled summary.
type SummaryOff uint8

// Summary-off representations accepted by Responses endpoints.
const (
	// SummaryOmitted indicates that the summary field is omitted.
	SummaryOmitted SummaryOff = iota
	// SummaryAuto indicates that the summary is requested automatically.
	SummaryAuto
)

// ResponsesReasoning maps effort and adaptive thinking to a Responses request.
func ResponsesReasoning(thinking core.ThinkingConfig, off SummaryOff) *Reasoning {
	auto := core.ThinkingSummary("auto")
	switch config := thinking.(type) {
	case *core.BudgetConfig, *core.DisabledConfig:
		return nil
	case *core.AdaptiveConfig:
		return &Reasoning{Effort: config.Effort, Summary: &auto}
	case *core.EffortConfig:
		result := &Reasoning{Effort: config.Effort}
		if config.Effort == "none" {
			return result
		}
		result.Summary = config.Summary
		if result.Summary == nil || *result.Summary == "off" && off == SummaryAuto {
			result.Summary = &auto
		}
		if result.Summary != nil && *result.Summary == "off" && off == SummaryOmitted {
			result.Summary = nil
		}
		return result
	}
	return nil
}

// ResponsesTool is a tool declaration for the Responses endpoint.
type ResponsesTool struct {
	Type        string          `json:"type"`
	Name        string          `json:"name"`
	Description string          `json:"description"`
	Parameters  json.RawMessage `json:"parameters"`
	Strict      json.RawMessage `json:"strict,omitempty"`
}

// NewResponsesTool optionally includes strict:null, as Codex expects.
func NewResponsesTool(tool ToolDefinition, strictNull bool) ResponsesTool {
	result := ResponsesTool{
		Type:        "function",
		Name:        tool.Name,
		Description: tool.Description,
		Parameters:  tool.InputSchema,
	}
	if strictNull {
		result.Strict = json.RawMessage("null")
	}
	return result
}

// AssistantReplay controls the metadata on replayed assistant messages.
type AssistantReplay uint8

// Assistant replay shapes accepted by Responses endpoints.
const (
	// AssistantMinimal identifies assistant replay without metadata.
	AssistantMinimal AssistantReplay = iota
	// AssistantCompleted identifies assistant replay with completed status.
	AssistantCompleted
	// AssistantIdentified identifies assistant replay with an ID and completed status.
	AssistantIdentified
)

// ReasoningReplay controls whether unknown reasoning fields are replayed.
type ReasoningReplay uint8

// Reasoning replay shapes accepted by Responses endpoints.
const (
	// ReasoningReplayable identifies replay of known reasoning fields.
	ReasoningReplayable ReasoningReplay = iota
	// ReasoningWhole identifies replay of all reasoning fields.
	ReasoningWhole
)

// ToolMedia controls the placement of media returned by tools.
type ToolMedia uint8

// Tool-media placements accepted by Responses endpoints.
const (
	// ToolMediaFollowUp identifies tool media sent in a following user message.
	ToolMediaFollowUp ToolMedia = iota
	// ToolMediaInline identifies tool media sent inside the tool output.
	ToolMediaInline
)

// ResponsesDialect states one endpoint's replay requirements.
type ResponsesDialect struct {
	SignatureTag string
	Assistant    AssistantReplay
	Reasoning    ReasoningReplay
	ToolMedia    ToolMedia
}

// InputItem is one JSON-serializable item of a Responses request's input.
//
//sumtype:decl
type InputItem interface{ inputItem() }

// UserInput is a user message with typed input parts.
type UserInput struct {
	Role    string      `json:"role"`
	Content []InputPart `json:"content"`
}

// AssistantInput is a replayed assistant message.
type AssistantInput struct {
	Type    string       `json:"type"`
	Role    string       `json:"role"`
	ID      *string      `json:"id,omitempty"`
	Status  *string      `json:"status,omitempty"`
	Content []OutputText `json:"content"`
}

// OutputText is replayed text with an empty annotations list.
type OutputText struct {
	Type        string            `json:"type"`
	Text        string            `json:"text"`
	Annotations []json.RawMessage `json:"annotations"`
}

// FunctionCallInput is a replayed function invocation.
type FunctionCallInput struct {
	Type      string  `json:"type"`
	ID        *string `json:"id,omitempty"`
	CallID    string  `json:"call_id"`
	Name      string  `json:"name"`
	Arguments string  `json:"arguments"`
}

// FunctionCallOutputInput is a replayed tool result.
type FunctionCallOutputInput struct {
	Type   string     `json:"type"`
	CallID string     `json:"call_id"`
	Output ToolOutput `json:"output"`
}

func (*UserInput) inputItem()               {}
func (*AssistantInput) inputItem()          {}
func (*ReasoningItem) inputItem()           {}
func (*FunctionCallInput) inputItem()       {}
func (*FunctionCallOutputInput) inputItem() {}

// ToolOutput is text or typed media parts in a Responses tool result.
//
//sumtype:decl
type ToolOutput interface{ toolOutput() }

// ToolOutputText serializes as plain text.
type ToolOutputText string

func (*ToolOutputText) toolOutput() {}

// ToolOutputParts serializes as an array of input parts.
type ToolOutputParts []InputPart

func (*ToolOutputParts) toolOutput() {}

// InputPart is text, an image URL or an inline document.
//
//sumtype:decl
type InputPart interface{ inputPart() }

// InputText carries one text part.
type InputText struct {
	Type string `json:"type"`
	Text string `json:"text"`
}

// InputImage carries an image URL and detail preference.
type InputImage struct {
	Type     string `json:"type"`
	ImageURL string `json:"image_url"`
	Detail   string `json:"detail"`
}

// InputFile carries a PDF data URL and file name.
type InputFile struct {
	Type     string `json:"type"`
	Filename string `json:"filename"`
	FileData string `json:"file_data"`
}

func (*InputText) inputPart()  {}
func (*InputImage) inputPart() {}
func (*InputFile) inputPart()  {}

// ResponsesInput replays a transcript in the vendor's Responses dialect.
func ResponsesInput(items []InferenceItem, dialect ResponsesDialect) ([]InputItem, error) {
	input := make([]InputItem, 0, len(items))
	for index, item := range items {
		switch item := item.(type) {
		case *UserMessage:
			input = append(input, &UserInput{Role: "user", Content: responsesUserParts(item.Content)})
		case *UserSteer:
			input = append(input, &UserInput{Role: "user", Content: responsesUserParts(item.Content)})
		case *AssistantText:
			assistant := &AssistantInput{
				Type:    "message",
				Role:    "assistant",
				Content: []OutputText{{Type: "output_text", Text: item.Text, Annotations: []json.RawMessage{}}},
			}
			if dialect.Assistant != AssistantMinimal {
				status := "completed"
				assistant.Status = &status
			}
			if dialect.Assistant == AssistantIdentified {
				id := "msg_" + ShortHash(fmt.Sprintf("%d:%s:%s", index, item.ModelID, item.Text))
				assistant.ID = &id
			}
			input = append(input, assistant)
		case *AssistantThinking:
			if item.Signature == nil || !strings.HasPrefix(*item.Signature, dialect.SignatureTag) {
				continue
			}
			reasoning, err := DecodeUntagged[ReasoningItem](strings.TrimPrefix(*item.Signature, dialect.SignatureTag))
			if err != nil {
				continue
			}
			if dialect.Reasoning == ReasoningReplayable {
				reasoning.Extra = nil
			}
			input = append(input, &reasoning)
		case *AssistantRedactedThinking:
		case *ToolUse:
			call, id := SplitToolUseID(item.ToolUseID)
			arguments, err := ToolArguments(item.Input)
			if err != nil {
				return nil, err
			}
			input = append(
				input,
				&FunctionCallInput{
					Type:      "function_call",
					ID:        id,
					CallID:    call,
					Name:      item.ToolName,
					Arguments: arguments,
				},
			)
		case *ToolResult:
			call, _ := SplitToolUseID(item.ToolUseID)
			input = appendResponsesResult(input, call, item.Output, dialect.ToolMedia)
		}
	}
	return input, nil
}

// responsesUserParts converts resolved media to Responses input parts.
func responsesUserParts(content []UserPart) []InputPart {
	parts := make([]InputPart, 0, len(content))
	for _, part := range content {
		switch part := part.(type) {
		case *TextPart:
			parts = append(parts, &InputText{Type: "input_text", Text: part.Text})
		case *DocumentPart:
			parts = append(
				parts,
				&InputFile{Type: "input_file", Filename: part.FileName, FileData: dataURL(part.Bytes)},
			)
		case *ImagePart:
			parts = append(parts, &InputImage{Type: "input_image", ImageURL: mediaURL(part.Medium), Detail: "auto"})
		case *VideoPart:
			parts = append(parts, &InputImage{Type: "input_image", ImageURL: mediaURL(part.Medium), Detail: "auto"})
		}
	}
	return parts
}

// appendResponsesResult places a tool's media inline or in a following user message.
func appendResponsesResult(input []InputItem, call string, output []ResultPart, media ToolMedia) []InputItem {
	result := &FunctionCallOutputInput{Type: "function_call_output", CallID: call}
	if media == ToolMediaFollowUp {
		text := ToolOutputText(ToolOutputTextOf(output))
		result.Output = &text
		input = append(input, result)
		parts := make([]InputPart, 0)
		for _, part := range output {
			if bytes := resultMedia(part); bytes != nil {
				parts = append(parts, &InputImage{Type: "input_image", ImageURL: dataURL(*bytes), Detail: "auto"})
			}
		}
		if len(parts) > 0 {
			note := &InputText{Type: "input_text", Text: "[media returned by tool call " + call + "]"}
			input = append(input, &UserInput{Role: "user", Content: append([]InputPart{note}, parts...)})
		}
		return input
	}
	texts := make([]string, 0)
	images := make([]InputPart, 0)
	for _, part := range output {
		switch part := part.(type) {
		case *TextPart:
			texts = append(texts, part.Text)
		case *ResultImage:
			images = append(images, &InputImage{Type: "input_image", ImageURL: dataURL(part.Bytes), Detail: "auto"})
		case *ResultVideo:
		}
	}
	text := ToolOutputText(strings.Join(texts, "\n"))
	result.Output = &text
	if len(images) > 0 {
		parts := ToolOutputParts{}
		if text != "" {
			parts = append(parts, &InputText{Type: "input_text", Text: string(text)})
		}
		parts = append(parts, images...)
		result.Output = &parts
	}
	return append(input, result)
}

// ChatMessage is one message of a Chat Completions request.
//
//sumtype:decl
type ChatMessage interface{ chatMessage() }

// ChatSystem is the system prompt.
type ChatSystem struct {
	Role    string `json:"role"`
	Content string `json:"content"`
}

// ChatUser is a user message with text or media parts.
type ChatUser struct {
	Role    string      `json:"role"`
	Content ChatContent `json:"content"`
}

// ChatAssistant coalesces consecutive assistant text and calls.
type ChatAssistant struct {
	Role             string         `json:"role"`
	Content          *string        `json:"content"`
	ToolCalls        []ChatToolCall `json:"tool_calls,omitempty"`
	ReasoningContent *string        `json:"reasoning_content,omitempty"`
}

// ChatToolResult is a textual tool result.
type ChatToolResult struct {
	Role       string `json:"role"`
	ToolCallID string `json:"tool_call_id"`
	Content    string `json:"content"`
}

func (*ChatSystem) chatMessage()     {}
func (*ChatUser) chatMessage()       {}
func (*ChatAssistant) chatMessage()  {}
func (*ChatToolResult) chatMessage() {}

// ChatContent is plain text or typed content parts.
//
//sumtype:decl
type ChatContent interface{ chatContent() }

// ChatText serializes as plain text.
type ChatText string

func (*ChatText) chatContent() {}

// ChatParts serializes as content parts.
type ChatParts []ChatPart

func (*ChatParts) chatContent() {}

// ChatPart is text, an image URL or a file.
//
//sumtype:decl
type ChatPart interface{ chatPart() }

// ChatTextPart carries text in a multipart message.
type ChatTextPart struct {
	Type string `json:"type"`
	Text string `json:"text"`
}

// ChatImagePart carries an image URL.
type ChatImagePart struct {
	Type     string   `json:"type"`
	ImageURL ImageURL `json:"image_url"`
}

// ChatFilePart carries a PDF.
type ChatFilePart struct {
	Type string   `json:"type"`
	File FileData `json:"file"`
}

func (*ChatTextPart) chatPart()  {}
func (*ChatImagePart) chatPart() {}
func (*ChatFilePart) chatPart()  {}

// ImageURL is a Chat Completions image location.
type ImageURL struct {
	URL    string `json:"url"`
	Detail string `json:"detail"`
}

// FileData is a PDF data URL and original name.
type FileData struct {
	Filename string `json:"filename"`
	FileData string `json:"file_data"`
}

// ChatToolCall is a replayed Chat Completions function call.
type ChatToolCall struct {
	ID       string           `json:"id"`
	Type     string           `json:"type"`
	Function ChatFunctionCall `json:"function"`
}

// ChatFunctionCall is a function name and its serialized arguments.
type ChatFunctionCall struct {
	Name      string `json:"name"`
	Arguments string `json:"arguments"`
}

// ChatTool is a Chat Completions tool definition.
type ChatTool struct {
	Type     string       `json:"type"`
	Function ChatFunction `json:"function"`
}

// ChatFunction describes a function's input schema.
type ChatFunction struct {
	Name        string          `json:"name"`
	Description string          `json:"description"`
	Parameters  json.RawMessage `json:"parameters"`
}

// NewChatTool converts an inference tool definition.
func NewChatTool(tool ToolDefinition) ChatTool {
	return ChatTool{
		Type:     "function",
		Function: ChatFunction{Name: tool.Name, Description: tool.Description, Parameters: tool.InputSchema},
	}
}

// ChatMedia specifies whether a vendor takes native media or only images.
type ChatMedia uint8

// Media formats accepted by Chat endpoints.
const (
	// ChatNative identifies native media in chat messages.
	ChatNative ChatMedia = iota
	// ChatImages identifies image media with text placeholders for other media.
	ChatImages
)

// ChatDialect states one endpoint's transcript replay requirements.
type ChatDialect struct {
	ReasoningContent bool
	Media            ChatMedia
}

// ChatMessages replays the transcript, coalescing adjacent assistant text and calls.
func ChatMessages(systemPrompt string, items []InferenceItem, dialect ChatDialect) ([]ChatMessage, error) {
	messages := make([]ChatMessage, 0)
	if !core.IsBlank(systemPrompt) {
		messages = append(messages, &ChatSystem{Role: "system", Content: systemPrompt})
	}
	var open *ChatAssistant
	pending := ""
	reasoning := ""
	openTurn := func() *ChatAssistant {
		if open == nil {
			open = &ChatAssistant{Role: "assistant"}
			reasoning = pending
			pending = ""
		}
		return open
	}
	flush := func() {
		flushChatTurn(&messages, &open, &pending, &reasoning, dialect)
	}
	for _, item := range items {
		switch item := item.(type) {
		case *UserMessage:
			flush()
			messages = append(messages, &ChatUser{Role: "user", Content: chatUserContent(item.Content, dialect.Media)})
		case *UserSteer:
			flush()
			messages = append(messages, &ChatUser{Role: "user", Content: chatUserContent(item.Content, dialect.Media)})
		case *AssistantText:
			appendAssistantText(openTurn(), item.Text)
		case *ToolUse:
			arguments, err := ToolArguments(item.Input)
			if err != nil {
				return nil, err
			}
			turn := openTurn()
			turn.ToolCalls = append(
				turn.ToolCalls,
				ChatToolCall{
					ID:       item.ToolUseID,
					Type:     "function",
					Function: ChatFunctionCall{Name: item.ToolName, Arguments: arguments},
				},
			)
		case *ToolResult:
			flush()
			messages = append(
				messages,
				&ChatToolResult{Role: "tool", ToolCallID: item.ToolUseID, Content: ToolOutputTextOf(item.Output)},
			)
			messages = append(messages, chatResultMedia(item, dialect)...)
		case *AssistantThinking:
			appendChatThinking(item.Text, dialect, open, &pending, &reasoning)
		case *AssistantRedactedThinking:
		}
	}
	flush()
	return messages, nil
}

// chatUserContent uses plain text when every emitted part is text.
func chatUserContent(content []UserPart, media ChatMedia) ChatContent {
	parts := ChatParts{}
	for _, part := range content {
		switch part := part.(type) {
		case *TextPart:
			parts = append(parts, &ChatTextPart{Type: "text", Text: part.Text})
		case *DocumentPart:
			if media == ChatNative {
				parts = append(
					parts,
					&ChatFilePart{Type: "file", File: FileData{Filename: part.FileName, FileData: dataURL(part.Bytes)}},
				)
			}
		case *VideoPart:
			if media == ChatImages {
				named := ""
				switch medium := part.Medium.(type) {
				case *MediaBytes:
					named = medium.MediaType
				case *MediaURL:
					named = medium.URL
				}
				parts = append(parts, &ChatTextPart{Type: "text", Text: "[video:" + named + "]"})
			} else {
				parts = append(
					parts,
					&ChatImagePart{Type: "image_url", ImageURL: ImageURL{URL: mediaURL(part.Medium), Detail: "auto"}},
				)
			}
		case *ImagePart:
			parts = append(
				parts,
				&ChatImagePart{Type: "image_url", ImageURL: ImageURL{URL: mediaURL(part.Medium), Detail: "auto"}},
			)
		}
	}
	texts := make([]string, 0, len(parts))
	for _, part := range parts {
		switch part := part.(type) {
		case *ChatTextPart:
			texts = append(texts, part.Text)
		case *ChatImagePart, *ChatFilePart:
			return &parts
		}
	}
	text := ChatText(strings.Join(texts, "\n"))
	return &text
}

// ToolOutputTextOf renders tool media as placeholders when output is text-only.
func ToolOutputTextOf(output []ResultPart) string {
	parts := make([]string, 0, len(output))
	for _, part := range output {
		switch part := part.(type) {
		case *TextPart:
			parts = append(parts, part.Text)
		case *ResultImage:
			parts = append(parts, "[image:"+part.Bytes.MediaType+"]")
		case *ResultVideo:
			parts = append(parts, "[video:"+part.Bytes.MediaType+"]")
		}
	}
	return strings.Join(parts, "\n")
}

// ToolArguments returns a string unchanged, null as {}, and any other input as JSON.
func ToolArguments(input json.RawMessage) (string, error) {
	if len(input) == 0 || contract.IsNull(input) {
		return "{}", nil
	}
	value, err := canonicalVendorJSON(input, 0)
	if err != nil {
		return "", &WireError{Field: ".", Err: err}
	}
	var text string
	if err := json.Unmarshal(value, &text); err == nil {
		return text, nil
	}
	return string(value), nil
}

// resultMedia extracts the bytes of a tool's returned image or video.
func resultMedia(part ResultPart) *MediaBytes {
	switch part := part.(type) {
	case *ResultImage:
		return &part.Bytes
	case *ResultVideo:
		return &part.Bytes
	case *TextPart:
		return nil
	}
	return nil
}

// mediaURL resolves the URL representation of a transcript medium.
func mediaURL(medium Medium) string {
	switch medium := medium.(type) {
	case *MediaBytes:
		return dataURL(*medium)
	case *MediaURL:
		return medium.URL
	}
	return ""
}

// dataURL encodes a resolved medium for vendor requests.
func dataURL(bytes MediaBytes) string {
	return "data:" + bytes.MediaType + ";base64," + base64.StdEncoding.EncodeToString(bytes.Data)
}

func chatResultMedia(item *ToolResult, dialect ChatDialect) []ChatMessage {
	var messages []ChatMessage
	if dialect.Media == ChatNative {
		parts := ChatParts{}
		for _, part := range item.Output {
			if bytes := resultMedia(part); bytes != nil {
				parts = append(
					parts,
					&ChatImagePart{Type: "image_url", ImageURL: ImageURL{URL: dataURL(*bytes), Detail: "auto"}},
				)
			}
		}
		if len(parts) > 0 {
			parts = append(
				ChatParts{
					&ChatTextPart{Type: "text", Text: "[media returned by tool call " +
						item.ToolUseID +
						"]"},
				},
				parts...)
			messages = append(messages, &ChatUser{Role: "user", Content: &parts})
		}
	}
	return messages
}

func flushChatTurn(messages *[]ChatMessage, open **ChatAssistant, pending, reasoning *string, dialect ChatDialect) {
	*pending = ""
	if *open == nil {
		return
	}
	if dialect.ReasoningContent && (*reasoning != "" || len((*open).ToolCalls) > 0) {
		value := *reasoning
		(*open).ReasoningContent = &value
	}
	*messages = append(*messages, *open)
	*open = nil
	*reasoning = ""
}

func appendAssistantText(turn *ChatAssistant, text string) {
	if turn.Content != nil {
		text = *turn.Content + text
	}
	if text != "" {
		turn.Content = &text
	}
}

func appendChatThinking(text string, dialect ChatDialect, open *ChatAssistant, pending, reasoning *string) {
	if dialect.ReasoningContent {
		if open == nil {
			*pending += text
		} else {
			*reasoning += text
		}
	}
}
