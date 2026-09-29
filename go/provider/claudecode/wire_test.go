package claudecode

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

// Cost: in-memory CLI transcripts only.
func TestCLITranscriptAndNewUserMessages(t *testing.T) {
	items := []provider.InferenceItem{
		provider.UserMessage{Content: []provider.UserPart{provider.TextPart{Text: "first"}}},
		provider.AssistantThinking{Text: "hidden", Signature: new("signature")},
		provider.AssistantText{Text: "answer"},
		provider.ToolUse{ToolUseID: "call", ToolName: "read", Input: jsontext.Value(`{"path":"file"}`)},
		provider.ToolResult{ToolUseID: "call", Output: []provider.ResultPart{provider.TextPart{Text: "denied"}}, IsError: true},
		provider.UserSteer{Content: []provider.UserPart{provider.ImagePart{Medium: provider.MediaURL{URL: "https://example.test/image"}}, provider.DocumentPart{Bytes: provider.MediaBytes{Data: core.NewB64Bytes([]byte("pdf")), MediaType: "application/pdf"}, FileName: "file.pdf"}, provider.VideoPart{Medium: provider.MediaURL{URL: "https://example.test/video"}}, provider.TextPart{Text: "next"}}},
	}
	raw := transcript(items)
	var line userInput
	if err := json.Unmarshal(raw, &line); err != nil {
		t.Fatal(err)
	}
	if line.Message.Role != "user" || len(line.Message.Content) != 4 {
		t.Fatalf("transcript %s", raw)
	}
	text := *line.Message.Content[0].Text
	want := "User: first\n\nAssistant: answer\n\n[Earlier in this conversation I called the tool read with input: {\"path\":\"file\"}.\n\nIt returned an error from read: denied]\n\nUser:"
	if text != want || strings.Contains(string(raw), "hidden") {
		t.Fatalf("transcript text %q", text)
	}
	if line.Message.Content[1].Type != "image" || line.Message.Content[2].Type != "document" || *line.Message.Content[3].Text != "[video]\n\nnext" {
		t.Fatalf("media %s", raw)
	}
	fresh := transcript(items[:1])
	if strings.Contains(string(fresh), "User:") {
		t.Fatal("single user gains a speaker prefix")
	}
	newLines := newUserMessages(items, 1)
	if strings.Contains(string(newLines), "first") || !strings.Contains(string(newLines), "next") || len(newUserMessages(items, 2)) != 0 {
		t.Fatalf("new messages %s", newLines)
	}
}
func TestCLIOutputEventsAndLastCallUsage(t *testing.T) {
	raw := []byte(`{"type":"assistant","message":{"content":[{"type":"future","text":12},{"type":"text","text":"hello","input":false},{"type":"thinking","text":"thought","signature":"signed"},{"type":"redacted_thinking","data":"redacted"},{"type":"tool_use","id":12,"name":"mcp__main__read","input":null}]}}`)
	line, known, err := decodeLine(raw)
	if err != nil || !known {
		t.Fatalf("decode %v", err)
	}
	assistant := line.(assistantLine)
	events := []provider.ProviderEvent{}
	var call provider.ToolCall
	for _, raw := range *assistant.Message.Content {
		b, known, err := block(raw)
		if err != nil {
			t.Fatal(err)
		}
		if !known {
			continue
		}
		if b.Type == "tool_use" {
			call, err = b.call()
			if err != nil {
				t.Fatal(err)
			}
		} else {
			events = append(events, b.events()...)
		}
	}
	want := []provider.ProviderEvent{provider.TextDelta{Text: "hello"}, provider.ThinkingStart{}, provider.ThinkingDelta{Text: "thought"}, provider.ThinkingSignature{Signature: "signed"}, provider.RedactedThinking{Data: "redacted"}}
	if !reflect.DeepEqual(events, want) || call.ToolUseID != "12" || call.ToolName != "read" || string(call.Input) != "{}" {
		t.Fatalf("events %#v call %#v", events, call)
	}
	line, _, err = decodeLine([]byte(`{"type":"result","usage":{"input_tokens":100,"iterations":[{"inputTokens":7,"input_tokens":8,"output_tokens":2,"cacheReadTokens":3,"cache_creation_input_tokens":4}]}}`))
	if err != nil {
		t.Fatal(err)
	}
	if got := line.(resultLine).end().Usage; got != (core.TokenUsage{InputTokens: 8, OutputTokens: 2, CacheReadTokens: 3, CacheWriteTokens: 4}) {
		t.Fatalf("usage %#v", got)
	}
	line, _, err = decodeLine([]byte(`{"type":"result","is_error":true,"result":" API Error: 400 ","errors":[{}," rejected "],"api_error_status":400}`))
	if err != nil {
		t.Fatal(err)
	}
	end := line.(resultLine).end()
	if end.Failure == nil || end.Failure.Message != "API Error: 400\nrejected" || end.Failure.Code != "" {
		t.Fatalf("failure %#v", end)
	}
	for _, raw := range []string{`{"type":"future","message":12}`, `{"type":"assistant","error":false}`} {
		if _, _, err := decodeLine([]byte(raw)); err != nil {
			t.Fatalf("optional line %s: %v", raw, err)
		}
	}
	for _, raw := range []string{`{}`, `{"type":"result","usage":{"input_tokens":-1}}`, `{"type":"assistant","message":[]}`, `{"type":"control_request","request_id":"one"}`} {
		if _, _, err := decodeLine([]byte(raw)); err == nil {
			t.Fatalf("malformed line accepted %s", raw)
		}
	}
	for _, raw := range []string{`{"type":"text","text":12}`, `{"type":"tool_use","id":false}`} {
		if _, _, err := block(jsontext.Value(raw)); err == nil {
			t.Fatalf("malformed block accepted %s", raw)
		}
	}
}

// Cost: in-memory CLI lines; process ownership and whole-message suppression wait for the process runtime.
func TestCLIStreamingAndFailureDecoding(t *testing.T) {
	var events []provider.ProviderEvent
	for _, raw := range []string{
		`{"type":"message_start"}`,
		`{"type":"content_block_start","content_block":{"type":"thinking"}}`,
		`{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"hmm"}}`,
		`{"type":"content_block_delta","delta":{"type":"thinking_delta"}}`,
		`{"type":"content_block_delta","delta":{"type":"signature_delta","signature":"sig"}}`,
		`{"type":"content_block_start","content_block":{"type":"text","text":""}}`,
		`{"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}`,
		`{"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{"}}`,
		`{"type":"future","delta":false}`,
	} {
		line, known, err := decodeLine([]byte(`{"type":"stream_event","event":` + raw + `}`))
		if err != nil || !known {
			t.Fatalf("stream line: %v", err)
		}
		stream, known, err := decodeStream(*line.(streamEventLine).Event)
		if err != nil {
			t.Fatal(err)
		}
		if !known {
			continue
		}
		next, err := stream.events()
		if err != nil {
			t.Fatal(err)
		}
		events = append(events, next...)
	}
	want := []provider.ProviderEvent{provider.ThinkingStart{}, provider.ThinkingDelta{Text: "hmm"}, provider.ThinkingSignature{Signature: "sig"}, provider.TextDelta{Text: "hi"}}
	if !reflect.DeepEqual(events, want) {
		t.Fatalf("stream events: %#v", events)
	}
	for _, raw := range []string{
		`{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":null}}}`,
		`{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":{}}}}`,
	} {
		if _, _, err := decodeLine([]byte(raw)); err == nil || !strings.Contains(err.Error(), "text") {
			t.Fatalf("malformed stream line: %v", err)
		}
	}
	for _, tc := range []struct {
		raw, message string
		code         provider.ErrorCode
	}{
		{`{"type":"error","message":"rate limited, try later"}`, "rate limited, try later", provider.RateLimit},
		{`{"type":"error","message":"boom","code":"auth_expired"}`, "boom", provider.AuthExpired},
		{`{"type":"error","message":{}}`, "Claude Code error", ""},
		{`{"type":"result","is_error":true,"result":"authentication failed","errors":[3]}`, "authentication failed", provider.AuthExpired},
		{`{"type":"result","is_error":true,"result":{}}`, "Claude Code returned an error", ""},
		{`{"type":"result","is_error":true,"result":"too many requests","api_error_status":429}`, "too many requests", provider.RateLimit},
	} {
		line, _, err := decodeLine([]byte(tc.raw))
		if err != nil {
			t.Fatal(err)
		}
		var failure provider.ProviderFailure
		switch line := line.(type) {
		case errorLine:
			failure = line.failure()
		case resultLine:
			end := line.end()
			if end.Failure == nil {
				t.Fatal("missing CLI failure")
			}
			failure = *end.Failure
		}
		if failure.Message != tc.message || failure.Code != tc.code {
			t.Fatalf("CLI failure: %#v", failure)
		}
	}
}

// Cost: in-memory CLI lines; IDs must match serde's spelling so tool results find their calls.
func TestCLINumericToolIDsUseSerdeNumberSpelling(t *testing.T) {
	for _, tc := range []struct{ raw, want string }{
		{`12`, "12"},
		{`-12`, "-12"},
		{`18446744073709551615`, "18446744073709551615"},
		{`-9223372036854775808`, "-9223372036854775808"},
		{`18446744073709551616`, "1.8446744073709552e19"},
		{`1e2`, "100.0"},
		{`1.2300`, "1.23"},
		{`1e+020`, "1e20"},
		{`1e-007`, "1e-7"},
		{`-0.0`, "-0.0"},
		{`-0`, "-0.0"},
		{`"1e2"`, "1e2"},
	} {
		t.Run(tc.raw, func(t *testing.T) {
			line, known, err := decodeLine([]byte(`{"type":"assistant","message":{"content":[{"type":"tool_use","id":` + tc.raw + `,"name":"read","input":{}}]}}`))
			if err != nil || !known {
				t.Fatalf("CLI line: %v", err)
			}
			content := *line.(assistantLine).Message.Content
			block, known, err := block(content[0])
			if err != nil || !known {
				t.Fatalf("tool block: %v", err)
			}
			call, err := block.call()
			if err != nil || call.ToolUseID != tc.want {
				t.Fatalf("tool ID = %q, want %q; error: %v", call.ToolUseID, tc.want, err)
			}
		})
	}
	if _, _, err := decodeLine([]byte(`{"type":"assistant","message":{"content":[{"type":"tool_use","id":1e400,"name":"read"}]}}`)); err == nil || !strings.Contains(err.Error(), "message.content[0].id") {
		t.Fatalf("out-of-range numeric ID: %v", err)
	}
}
