package claudecode

import (
	"encoding/base64"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
)

func spawnRequest(site Site, r provider.InferenceRequest, token provider.Secret) host.SpawnRequest {
	args := []string{"--print", "--output-format", "stream-json", "--verbose", "--input-format", "stream-json", "--include-partial-messages", "--no-session-persistence", "--safe-mode", "--disable-slash-commands", "--tools", "", "--permission-mode", "bypassPermissions", "--allow-dangerously-skip-permissions", "--model", r.ModelID, "--system-prompt", r.SystemPrompt}
	if effort := provider.ReasoningEffort(r.Thinking); effort != nil {
		args = append(args, "--effort", *effort)
	}
	values := make(map[string]*string)
	for key, value := range map[string]string{"CLAUDE_CODE_OAUTH_TOKEN": token.Expose(), "CLAUDE_CONFIG_DIR": site.ConfigDir, "DISABLE_AUTOUPDATER": "1", "DISABLE_AUTO_COMPACT": "1", "MAX_MCP_OUTPUT_TOKENS": "1000000"} {
		values[key] = &value
	}
	values["CLAUDECODE"] = nil
	return host.SpawnRequest{Command: site.Executable, Args: args, CWD: &site.RunDir, Env: host.SpawnEnv{Mode: host.Overlay, Values: values}, Retained: true}
}

type inputBlock struct {
	Type   string `json:"type"`
	Source any    `json:"source"`
}

type inputText struct {
	Type string `json:"type"`
	Text string `json:"text"`
}

// textBlock keeps empty text present on the wire too.
func textBlock(text string) any { return inputText{"text", text} }
func userContent(parts []provider.UserPart) []any {
	blocks := make([]any, 0, len(parts))
	for _, part := range parts {
		switch p := part.(type) {
		case *provider.TextPart:
			blocks = append(blocks, textBlock(p.Text))
		case *provider.VideoPart:
			blocks = append(blocks, textBlock("[video]"))
		case *provider.ImagePart:
			var source any
			switch m := p.Medium.(type) {
			case *provider.MediaBytes:
				source = mediaSource(*m)
			case *provider.MediaURL:
				source = map[string]any{"type": "url", "url": m.URL}
			}
			blocks = append(blocks, inputBlock{Type: "image", Source: source})
		case *provider.DocumentPart:
			blocks = append(blocks, map[string]any{"type": "document", "source": mediaSource(p.Bytes), "title": p.FileName})
		}
	}
	return blocks
}
func mediaSource(b provider.MediaBytes) any {
	return map[string]any{"type": "base64", "media_type": b.MediaType, "data": base64.StdEncoding.EncodeToString(b.Data)}
}
func userLine(content []any) any {
	return map[string]any{"type": "user", "message": map[string]any{"role": "user", "content": content}}
}
func userMessages(items []provider.InferenceItem) [][]provider.UserPart {
	var result [][]provider.UserPart
	for _, item := range items {
		switch i := item.(type) {
		case *provider.UserMessage:
			result = append(result, i.Content)
		case *provider.UserSteer:
			result = append(result, i.Content)
		case *provider.AssistantText, *provider.AssistantThinking, *provider.AssistantRedactedThinking, *provider.ToolUse, *provider.ToolResult:
		}
	}
	return result
}

type transcriptPart struct {
	role   string
	blocks []any
}

func transcript(items []provider.InferenceItem) any {
	var parts []transcriptPart
	add := func(role string, blocks []any) {
		if len(blocks) == 0 {
			return
		}
		if len(parts) > 0 && parts[len(parts)-1].role == role {
			parts[len(parts)-1].blocks = append(parts[len(parts)-1].blocks, blocks...)
			return
		}
		parts = append(parts, transcriptPart{role, blocks})
	}
	for _, item := range items {
		switch i := item.(type) {
		case *provider.UserMessage:
			add("User:", userContent(i.Content))
		case *provider.UserSteer:
			add("User:", userContent(i.Content))
		case *provider.AssistantText:
			if i.Text != "" {
				add("Assistant:", []any{textBlock(i.Text)})
			}
		case *provider.ToolUse:
			add("Assistant:", []any{textBlock(fmt.Sprintf("[Earlier in this conversation I called the tool %s with input: %s.", i.ToolName, i.Input))})
		case *provider.ToolResult:
			from := ""
			for _, earlier := range items {
				if call, ok := earlier.(*provider.ToolUse); ok && call.ToolUseID == i.ToolUseID {
					from = " from " + call.ToolName
					break
				}
			}
			prefix := "It returned"
			if i.IsError {
				prefix += " an error"
			}
			add("Assistant:", []any{textBlock(prefix + from + ": " + provider.ToolOutputTextOf(i.Output) + "]")})
		case *provider.AssistantThinking, *provider.AssistantRedactedThinking:
		}
	}
	if len(parts) == 1 && parts[0].role == "User:" {
		return userLine(parts[0].blocks)
	}
	blocks := make([]any, 0)
	var text strings.Builder
	for _, part := range parts {
		if text.Len() > 0 {
			text.WriteString("\n\n")
		}
		text.WriteString(part.role)
		named := true
		for _, block := range part.blocks {
			piece, ok := block.(inputText)
			if !ok {
				if text.Len() > 0 {
					blocks = append(blocks, textBlock(text.String()))
					text.Reset()
				}
				blocks = append(blocks, block)
				named = false
				continue
			}
			if named {
				text.WriteByte(' ')
			} else if text.Len() > 0 {
				text.WriteString("\n\n")
			}
			text.WriteString(piece.Text)
			named = false
		}
	}
	if text.Len() > 0 {
		blocks = append(blocks, textBlock(text.String()))
	}
	return userLine(blocks)
}
