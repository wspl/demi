package claudecode

import (
	"encoding/json/jsontext"
	"fmt"
	"strings"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

//demi:wire
type userInput struct {
	Type    string       `json:"type" check:"eq=user"`
	Message inputMessage `json:"message"`
}

//demi:wire
type inputMessage struct {
	Role    string       `json:"role"`
	Content []inputBlock `json:"content"`
}

//demi:wire
type inputBlock struct {
	Type   string       `json:"type"`
	Text   *string      `json:"text,omitzero"`
	Source *inputSource `json:"source,omitzero"`
	Title  *string      `json:"title,omitzero"`
}

//demi:wire
type inputSource struct {
	Type      string         `json:"type"`
	MediaType *string        `json:"media_type,omitzero"`
	Data      *core.B64Bytes `json:"data,omitzero" check:"func=core.Validate"`
	URL       *string        `json:"url,omitzero"`
}

//demi:wire
type initializeInput struct {
	Type      string            `json:"type" check:"eq=control_request"`
	RequestID string            `json:"request_id"`
	Request   initializeRequest `json:"request"`
}

//demi:wire
type initializeRequest struct {
	Subtype       string   `json:"subtype" check:"eq=initialize"`
	SDKMCPServers []string `json:"sdkMcpServers"`
	SystemPrompt  string   `json:"systemPrompt"`
}

// inputLine is one complete stream-json message to the CLI.
func inputLine(value any) []byte       { return append(provider.JSONBody(value), '\n') }
func textBlock(text string) inputBlock { return inputBlock{Type: "text", Text: &text} }
func userContent(parts []provider.UserPart) []inputBlock {
	blocks := make([]inputBlock, 0, len(parts))
	for _, part := range parts {
		switch part := part.(type) {
		case provider.TextPart:
			blocks = append(blocks, textBlock(part.Text))
		case provider.ImagePart:
			source := inputSource{}
			switch medium := part.Medium.(type) {
			case provider.MediaBytes:
				source = inputSource{Type: "base64", MediaType: &medium.MediaType, Data: &medium.Data}
			case provider.MediaURL:
				source = inputSource{Type: "url", URL: &medium.URL}
			}
			blocks = append(blocks, inputBlock{Type: "image", Source: &source})
		case provider.VideoPart:
			blocks = append(blocks, textBlock("[video]"))
		case provider.DocumentPart:
			blocks = append(blocks, inputBlock{Type: "document", Title: &part.FileName, Source: &inputSource{Type: "base64", MediaType: &part.Bytes.MediaType, Data: &part.Bytes.Data}})
		}
	}
	return blocks
}
func transcript(items []provider.InferenceItem) []byte {
	parts := []inputMessage{}
	appendPart := func(role string, blocks []inputBlock) {
		if len(blocks) == 0 {
			return
		}
		if len(parts) > 0 && parts[len(parts)-1].Role == role {
			parts[len(parts)-1].Content = append(parts[len(parts)-1].Content, blocks...)
		} else {
			parts = append(parts, inputMessage{Role: role, Content: blocks})
		}
	}
	for _, item := range items {
		switch item := item.(type) {
		case provider.UserMessage:
			appendPart("user", userContent(item.Content))
		case provider.UserSteer:
			appendPart("user", userContent(item.Content))
		case provider.AssistantText:
			if item.Text != "" {
				appendPart("assistant", []inputBlock{textBlock(item.Text)})
			}
		case provider.ToolUse:
			raw := item.Input
			if len(raw) == 0 {
				raw = jsontext.Value(`null`)
			}
			appendPart("assistant", []inputBlock{textBlock(fmt.Sprintf("[Earlier in this conversation I called the tool %s with input: %s.", item.ToolName, raw))})
		case provider.ToolResult:
			from := ""
			for _, earlier := range items {
				if call, ok := earlier.(provider.ToolUse); ok && call.ToolUseID == item.ToolUseID {
					from = " from " + call.ToolName
					break
				}
			}
			prefix := "It returned"
			if item.IsError {
				prefix += " an error"
			}
			appendPart("assistant", []inputBlock{textBlock(prefix + from + ": " + provider.ToolOutputText(item.Output) + "]")})
		}
	}
	content := []inputBlock{}
	if len(parts) == 1 && parts[0].Role == "user" {
		content = parts[0].Content
	} else {
		var text strings.Builder
		for _, part := range parts {
			if text.Len() > 0 {
				text.WriteString("\n\n")
			}
			if part.Role == "user" {
				text.WriteString("User:")
			} else {
				text.WriteString("Assistant:")
			}
			named := true
			for _, block := range part.Content {
				if block.Type != "text" {
					if text.Len() > 0 {
						content = append(content, textBlock(text.String()))
						text.Reset()
					}
					content = append(content, block)
					named = false
					continue
				}
				if named {
					text.WriteByte(' ')
				} else if text.Len() > 0 {
					text.WriteString("\n\n")
				}
				text.WriteString(*block.Text)
				named = false
			}
		}
		if text.Len() > 0 {
			content = append(content, textBlock(text.String()))
		}
	}
	return inputLine(userInput{Type: "user", Message: inputMessage{Role: "user", Content: content}})
}
func newUserMessages(items []provider.InferenceItem, sent int) []byte {
	lines := []byte{}
	count := 0
	for _, item := range items {
		var content []provider.UserPart
		switch item := item.(type) {
		case provider.UserMessage:
			content = item.Content
		case provider.UserSteer:
			content = item.Content
		default:
			continue
		}
		count++
		if count <= sent {
			continue
		}
		lines = append(lines, inputLine(userInput{Type: "user", Message: inputMessage{Role: "user", Content: userContent(content)}})...)
	}
	return lines
}
