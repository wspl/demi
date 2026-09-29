package provider

import (
	"encoding/json/jsontext"
	"fmt"
	"strings"

	"github.com/wspl/demi/go/core"
)

type ChatDialect struct {
	ReasoningContent bool
	ImagesOnly       bool
}

//demi:wire
type ChatSystemMessage struct {
	Role    string `json:"role" check:"eq=system"`
	Content string `json:"content"`
}

//demi:wire
type ChatUserMessage struct {
	Role    string         `json:"role" check:"eq=user"`
	Content jsontext.Value `json:"content"`
}

//demi:wire
type ChatAssistantMessage struct {
	Role             string                 `json:"role" check:"eq=assistant"`
	Content          *string                `json:"content" check:"nullable"`
	ToolCalls        *[]ChatRequestToolCall `json:"tool_calls,omitzero"`
	ReasoningContent *string                `json:"reasoning_content,omitzero"`
}

//demi:wire
type ChatToolMessage struct {
	Role       string `json:"role" check:"eq=tool"`
	ToolCallID string `json:"tool_call_id"`
	Content    string `json:"content"`
}

//demi:wire
type ChatRequestToolCall struct {
	ID       string                  `json:"id"`
	Type     string                  `json:"type" check:"eq=function"`
	Function ChatRequestFunctionCall `json:"function"`
}

//demi:wire
type ChatRequestFunctionCall struct {
	Name      string `json:"name"`
	Arguments string `json:"arguments"`
}

//demi:wire
type ChatTool struct {
	Type     string       `json:"type" check:"eq=function"`
	Function ChatFunction `json:"function"`
}

//demi:wire
type ChatFunction struct {
	Name        string         `json:"name"`
	Description string         `json:"description"`
	Parameters  jsontext.Value `json:"parameters"`
}

func NewChatTool(tool ToolDefinition) ChatTool {
	return ChatTool{Type: "function", Function: ChatFunction{Name: tool.Name, Description: tool.Description, Parameters: tool.InputSchema}}
}

//demi:wire
type ChatTextPart struct {
	Type string `json:"type" check:"eq=text"`
	Text string `json:"text"`
}

//demi:wire
type ChatImagePart struct {
	Type     string       `json:"type" check:"eq=image_url"`
	ImageURL ChatImageURL `json:"image_url"`
}

//demi:wire
type ChatImageURL struct {
	URL    string `json:"url"`
	Detail string `json:"detail" check:"eq=auto"`
}

//demi:wire
type ChatFilePart struct {
	Type string       `json:"type" check:"eq=file"`
	File ChatFileData `json:"file"`
}

//demi:wire
type ChatFileData struct {
	Filename string `json:"filename"`
	FileData string `json:"file_data"`
}

func ChatMessages(system string, items []InferenceItem, dialect ChatDialect) []jsontext.Value {
	messages := []jsontext.Value{}
	if !core.IsBlank(system) {
		messages = append(messages, JSONBody(ChatSystemMessage{Role: "system", Content: system}))
	}
	var open *ChatAssistantMessage
	pending := ""
	openTurn := func() *ChatAssistantMessage {
		if open == nil {
			open = &ChatAssistantMessage{Role: "assistant"}
			if pending != "" {
				value := pending
				open.ReasoningContent = &value
				pending = ""
			}
		}
		return open
	}
	flush := func() {
		pending = ""
		if open == nil {
			return
		}
		if dialect.ReasoningContent && open.ToolCalls != nil && open.ReasoningContent == nil {
			value := ""
			open.ReasoningContent = &value
		}
		messages = append(messages, JSONBody(open))
		open = nil
	}
	for _, item := range items {
		switch item := item.(type) {
		case UserMessage:
			flush()
			messages = append(messages, JSONBody(ChatUserMessage{Role: "user", Content: chatUserContent(item.Content, dialect.ImagesOnly)}))
		case UserSteer:
			flush()
			messages = append(messages, JSONBody(ChatUserMessage{Role: "user", Content: chatUserContent(item.Content, dialect.ImagesOnly)}))
		case AssistantText:
			turn := openTurn()
			text := item.Text
			if turn.Content != nil {
				text = *turn.Content + text
			}
			if text != "" {
				turn.Content = &text
			}
		case AssistantThinking:
			if dialect.ReasoningContent {
				if open == nil {
					pending += item.Text
				} else {
					text := item.Text
					if open.ReasoningContent != nil {
						text = *open.ReasoningContent + text
					}
					if text != "" {
						open.ReasoningContent = &text
					}
				}
			}
		case ToolUse:
			turn := openTurn()
			if turn.ToolCalls == nil {
				turn.ToolCalls = new([]ChatRequestToolCall)
			}
			*turn.ToolCalls = append(*turn.ToolCalls, ChatRequestToolCall{ID: item.ToolUseID, Type: "function", Function: ChatRequestFunctionCall{Name: item.ToolName, Arguments: ToolArguments(item.Input)}})
		case ToolResult:
			flush()
			messages = append(messages, JSONBody(ChatToolMessage{Role: "tool", ToolCallID: item.ToolUseID, Content: ToolOutputText(item.Output)}))
			if !dialect.ImagesOnly {
				parts := []jsontext.Value{}
				for _, part := range item.Output {
					if bytes := resultMedia(part); bytes != nil {
						parts = append(parts, JSONBody(ChatImagePart{Type: "image_url", ImageURL: ChatImageURL{URL: DataURL(*bytes), Detail: "auto"}}))
					}
				}
				if len(parts) > 0 {
					parts = append([]jsontext.Value{JSONBody(ChatTextPart{Type: "text", Text: fmt.Sprintf("[media returned by tool call %s]", item.ToolUseID)})}, parts...)
					messages = append(messages, JSONBody(ChatUserMessage{Role: "user", Content: JSONBody(parts)}))
				}
			}
		}
	}
	flush()
	return messages
}
func chatUserContent(content []UserPart, imagesOnly bool) jsontext.Value {
	parts := []jsontext.Value{}
	texts := []string{}
	onlyText := true
	for _, part := range content {
		switch part := part.(type) {
		case TextPart:
			texts = append(texts, part.Text)
			parts = append(parts, JSONBody(ChatTextPart{Type: "text", Text: part.Text}))
		case DocumentPart:
			if !imagesOnly {
				onlyText = false
				parts = append(parts, JSONBody(ChatFilePart{Type: "file", File: ChatFileData{Filename: part.FileName, FileData: DataURL(part.Bytes)}}))
			}
		case ImagePart:
			onlyText = false
			parts = append(parts, JSONBody(ChatImagePart{Type: "image_url", ImageURL: ChatImageURL{URL: MediaURLText(part.Medium), Detail: "auto"}}))
		case VideoPart:
			if imagesOnly {
				name := ""
				switch medium := part.Medium.(type) {
				case MediaBytes:
					name = medium.MediaType
				case MediaURL:
					name = medium.URL
				}
				text := "[video:" + name + "]"
				texts = append(texts, text)
				parts = append(parts, JSONBody(ChatTextPart{Type: "text", Text: text}))
			} else {
				onlyText = false
				parts = append(parts, JSONBody(ChatImagePart{Type: "image_url", ImageURL: ChatImageURL{URL: MediaURLText(part.Medium), Detail: "auto"}}))
			}
		}
	}
	if onlyText {
		return JSONBody(strings.Join(texts, "\n"))
	}
	return JSONBody(parts)
}
