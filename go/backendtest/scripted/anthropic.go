package scripted

import (
	"encoding/json/v2"
	"fmt"
	"strings"
	"testing"
)

// Frame is one event of a Messages API stream.
type Frame = map[string]any

// Answer is a Messages API stream that answers deltas, in that many pieces, and
// reports input and output tokens.
func Answer(deltas []string, input, output int) *Response {
	return Message([][]Frame{TextBlock(0, deltas)}, "end_turn", Frame{"input_tokens": input, "output_tokens": 0}, output)
}

// ToolUse is a Messages API stream that calls the tool name with input.
func ToolUse(id, name string, input any) *Response {
	usage := Frame{"input_tokens": 1, "output_tokens": 0}
	return Message([][]Frame{ToolUseBlock(0, id, name, input)}, "tool_use", usage, 1)
}

// TextBlock is the frames that open a text block at index and fill it with
// deltas.
func TextBlock(index int, deltas []string) []Frame {
	frames := []Frame{{
		"type": "content_block_start", "index": index,
		"content_block": Frame{"type": "text", "text": ""},
	}}
	for _, delta := range deltas {
		frames = append(frames, Frame{
			"type": "content_block_delta", "index": index,
			"delta": Frame{"type": "text_delta", "text": delta},
		})
	}
	return frames
}

// ThinkingBlock is the frames that open a thinking block at index, stream text
// and sign it.
func ThinkingBlock(index int, text, signature string) []Frame {
	return []Frame{
		{
			"type": "content_block_start", "index": index,
			"content_block": Frame{"type": "thinking", "thinking": "", "signature": ""},
		},
		{
			"type": "content_block_delta", "index": index,
			"delta": Frame{"type": "thinking_delta", "thinking": text},
		},
		{
			"type": "content_block_delta", "index": index,
			"delta": Frame{"type": "signature_delta", "signature": signature},
		},
	}
}

// ToolUseBlock is the frames of a call of the tool name with input, at index.
func ToolUseBlock(index int, id, name string, input any) []Frame {
	encoded, err := json.Marshal(input)
	if err != nil {
		panic(fmt.Sprintf("a scripted tool input is JSON: %v", err))
	}
	return []Frame{
		{
			"type": "content_block_start", "index": index,
			"content_block": Frame{"type": "tool_use", "id": id, "name": name, "input": Frame{}},
		},
		{
			"type": "content_block_delta", "index": index,
			"delta": Frame{"type": "input_json_delta", "partial_json": string(encoded)},
		},
	}
}

// Message is one message of the content blocks whose frames blocks open and
// fill, each closed after its frames; usage is the message's usage at its
// start, and output its output tokens at its end.
func Message(blocks [][]Frame, stopReason string, usage Frame, output int) *Response {
	frames := []Frame{MessageStart(usage)}
	for _, block := range blocks {
		frames = append(frames, block...)
		frames = append(frames, Frame{"type": "content_block_stop", "index": block[0]["index"]})
	}
	frames = append(frames,
		Frame{"type": "message_delta", "delta": Frame{"stop_reason": stopReason}, "usage": Frame{"output_tokens": output}},
		Frame{"type": "message_stop"},
	)
	return EventStream(Events(frames))
}

// MessageStart is the frame that starts a message whose usage at its start is
// usage.
func MessageStart(usage Frame) Frame {
	return Frame{"type": "message_start", "message": Frame{
		"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-4-8", "content": []any{},
		"usage": usage,
	}}
}

// Events is frames as the text of a Messages API event stream.
func Events(frames []Frame) string {
	var text strings.Builder
	for _, frame := range frames {
		encoded, err := json.Marshal(frame)
		if err != nil {
			panic(fmt.Sprintf("a scripted frame is JSON: %v", err))
		}
		fmt.Fprintf(&text, "event: %s\ndata: %s\n\n", frame["type"], encoded)
	}
	return text.String()
}

// ToolResult is the text of the tool result id that a Messages API request
// carries.
func ToolResult(t testing.TB, request any, id string) string {
	t.Helper()
	messages, _ := request.(map[string]any)["messages"].([]any)
	for _, message := range messages {
		content, _ := message.(map[string]any)["content"].([]any)
		for _, block := range content {
			block, _ := block.(map[string]any)
			if block["type"] == "tool_result" && block["tool_use_id"] == id {
				parts, _ := block["content"].([]any)
				if len(parts) > 0 {
					text, _ := parts[0].(map[string]any)["text"].(string)
					return text
				}
			}
		}
	}
	t.Fatalf("no result of %s: %v", id, request)
	return ""
}
