package openaiapi_test

import (
	"context"
	"encoding/json"
	"os"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

// toolUse adds a read_file call to the test transcript.
func toolUse(id, path string) provider.InferenceItem {
	return &provider.ToolUse{ModelID: "gpt-test", ToolUseID: id, ToolName: "read_file", Input: json.RawMessage(`{"path":"` + path + `"}`)}
}

// toolResult adds the tool's text reply to the test transcript.
func toolResult(id, text string) provider.InferenceItem {
	return &provider.ToolResult{ToolUseID: id, Output: []provider.ResultPart{&provider.TextPart{Text: text}}}
}

func TestOutputLimits(t *testing.T) {
	for _, wire := range wires {
		field := "max_output_tokens"
		if wire == core.WireAPIChatCompletions {
			field = "max_completion_tokens"
		}
		limit := uint32(8000)
		capTokens := uint32(1024)
		if wire == core.WireAPIChatCompletions {
			limit = 32000
		}
		for _, tc := range []struct {
			limit, cap *uint32
			want       any
		}{{&limit, nil, float64(limit)}, {nil, nil, nil}, {&limit, &capTokens, float64(1024)}, {nil, &capTokens, float64(1024)}} {
			request := requestWith(user("hello"))
			request.OutputLimit = tc.limit
			request.OutputCap = tc.cap
			body := bodyOf(t.Context(), t, wire, provider.VendorPolicy{}, request)
			if body[field] != tc.want {
				t.Fatalf("%s: %v != %v", field, body[field], tc.want)
			}
			if _, found := body[field]; found && tc.want == nil {
				t.Fatalf("%s must be omitted", field)
			}
		}
	}
}

// transcriptBody checks the complete vendor request against the Rust fixture.
func transcriptBody(ctx context.Context, t *testing.T, wire core.WireAPI, fixture, id string) {
	t.Helper()
	request := requestWith(user("hello"), &provider.AssistantText{ModelID: "gpt-test", Text: "Use tool"}, toolUse(id, "a.ts"), toolResult(id, "contents"))
	request.SystemPrompt = "system instructions"
	tier := "priority"
	request.ServiceTierID = &tier
	request.Thinking = &core.EffortConfig{Effort: "high"}
	request.Tools = []provider.ToolDefinition{{Name: "read_file", Description: "Read a file", InputSchema: json.RawMessage(`{"type":"object","properties":{"path":{"type":"string"}}}`)}}
	want, err := os.ReadFile("testdata/" + fixture + ".json")
	if err != nil {
		t.Fatal(err)
	}
	assertJSON(t, bodyOf(ctx, t, wire, provider.VendorPolicy{}, request), string(want))
}

func TestResponsesTranscript(t *testing.T) {
	transcriptBody(t.Context(), t, core.WireAPIResponses, "responses", "call-1|fc-1")
}
func TestChatTranscript(t *testing.T) {
	transcriptBody(t.Context(), t, core.WireAPIChatCompletions, "chat", "call-1")
}

func TestOmittedSettings(t *testing.T) {
	request := requestWith(user("hello"))
	for _, blank := range []string{" \n", " \n\ufeff"} {
		request.SystemPrompt = blank
		body := bodyOf(t.Context(), t, core.WireAPIResponses, provider.VendorPolicy{}, request)
		for _, field := range []string{"instructions", "tools", "tool_choice", "parallel_tool_calls", "reasoning", "service_tier", "stream_options"} {
			if _, found := body[field]; found {
				t.Errorf("unexpected %s", field)
			}
		}
	}
	request.SystemPrompt = ""
	body := bodyOf(t.Context(), t, core.WireAPIChatCompletions, provider.VendorPolicy{}, request)
	assertJSON(t, body["messages"], `[{"role":"user","content":"hello"}]`)
	for _, field := range []string{"tools", "tool_choice", "reasoning_effort", "service_tier"} {
		if _, found := body[field]; found {
			t.Errorf("unexpected %s", field)
		}
	}
}

func TestReasoningSettings(t *testing.T) {
	off := core.ThinkingSummary("off")
	concise := core.ThinkingSummary("concise")
	cases := []struct {
		thinking core.ThinkingConfig
		want     string
	}{
		{&core.EffortConfig{Effort: "high", Summary: &off}, `{"effort":"high"}`},
		{&core.EffortConfig{Effort: "high"}, `{"effort":"high","summary":"auto"}`},
		{&core.EffortConfig{Effort: "low", Summary: &concise}, `{"effort":"low","summary":"concise"}`},
		{&core.EffortConfig{Effort: "none"}, `{"effort":"none"}`},
		{&core.AdaptiveConfig{Effort: "medium"}, `{"effort":"medium","summary":"auto"}`},
	}
	for _, tc := range cases {
		request := requestWith(user("hello"))
		request.Thinking = tc.thinking
		assertJSON(t, bodyOf(t.Context(), t, core.WireAPIResponses, provider.VendorPolicy{}, request)["reasoning"], tc.want)
	}
	for _, thinking := range []core.ThinkingConfig{&core.BudgetConfig{BudgetTokens: 2048}, &core.DisabledConfig{}} {
		request := requestWith(user("hello"))
		request.Thinking = thinking
		for _, wire := range wires {
			body := bodyOf(t.Context(), t, wire, provider.VendorPolicy{}, request)
			for _, field := range []string{"reasoning", "reasoning_effort"} {
				if _, found := body[field]; found {
					t.Errorf("unexpected %s", field)
				}
			}
		}
	}
}

func TestAssistantStatusPolicy(t *testing.T) {
	request := requestWith(user("hello"), &provider.AssistantText{ModelID: "gpt-test", Text: "prior reply"})
	for _, enabled := range []bool{false, true} {
		body := bodyOf(t.Context(), t, core.WireAPIResponses, provider.VendorPolicy{ReplayAssistantStatus: enabled}, request)
		input, ok := body["input"].([]any)
		if !ok || len(input) != 2 {
			t.Fatalf("input: %#v", body["input"])
		}
		want := `{"type":"message","role":"assistant","content":[{"type":"output_text","text":"prior reply","annotations":[]}]}`
		if enabled {
			want = `{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"prior reply","annotations":[]}]}`
		}
		assertJSON(t, input[1], want)
	}
}

func TestReasoningSignatureReplay(t *testing.T) {
	item := `{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"sum"}],"encrypted_content":"enc","status":"completed"}`
	items := []provider.InferenceItem{user("hello")}
	for _, signature := range []string{"openai:" + item, "codex:" + strings.Replace(item, "rs_1", "rs_foreign", 1), item, "", `openai:{"type":"message"}`} {
		thinking := &provider.AssistantThinking{ModelID: "gpt-test", Text: "thinking"}
		if signature != "" {
			thinking.Signature = &signature
		}
		items = append(items, thinking)
	}
	body := bodyOf(t.Context(), t, core.WireAPIResponses, provider.VendorPolicy{}, requestWith(items...))
	assertJSON(t, body["input"], `[{"role":"user","content":[{"type":"input_text","text":"hello"}]},{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"sum"}],"encrypted_content":"enc"}]`)
}

func TestUserMedia(t *testing.T) {
	request := requestWith(&provider.UserMessage{Content: []provider.UserPart{
		&provider.TextPart{Text: "look"},
		&provider.ImagePart{Medium: &provider.MediaBytes{Data: []byte("PNG"), MediaType: "image/png"}},
		&provider.ImagePart{Medium: &provider.MediaURL{URL: "https://example.com/a.png"}},
		&provider.DocumentPart{Bytes: provider.MediaBytes{Data: []byte("%PDF"), MediaType: "application/pdf"}, FileName: "a.pdf"},
	}})
	for _, wire := range wires {
		body := bodyOf(t.Context(), t, wire, provider.VendorPolicy{}, request)
		if wire == core.WireAPIResponses {
			assertJSON(t, body["input"], `[{"role":"user","content":[{"type":"input_text","text":"look"},{"type":"input_image","image_url":"data:image/png;base64,UE5H","detail":"auto"},{"type":"input_image","image_url":"https://example.com/a.png","detail":"auto"},{"type":"input_file","filename":"a.pdf","file_data":"data:application/pdf;base64,JVBERg=="}]}]`)
		} else {
			assertJSON(t, body["messages"], `[{"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"data:image/png;base64,UE5H","detail":"auto"}},{"type":"image_url","image_url":{"url":"https://example.com/a.png","detail":"auto"}},{"type":"file","file":{"filename":"a.pdf","file_data":"data:application/pdf;base64,JVBERg=="}}]}]`)
		}
	}
}

func TestToolMedia(t *testing.T) {
	output := []provider.ResultPart{
		&provider.TextPart{Text: "<binary stdout: 75 bytes>"},
		&provider.ResultImage{Bytes: provider.MediaBytes{Data: []byte("AAAA"), MediaType: "image/png"}},
		&provider.ResultVideo{Bytes: provider.MediaBytes{Data: []byte("MP4"), MediaType: "video/mp4"}},
	}
	for _, wire := range wires {
		id := "call-1|fc-1"
		field := "input"
		if wire == core.WireAPIChatCompletions {
			id = "call-1"
			field = "messages"
		}
		body := bodyOf(t.Context(), t, wire, provider.VendorPolicy{}, requestWith(toolUse(id, "shot.png"), &provider.ToolResult{ToolUseID: id, Output: output}))
		items, ok := body[field].([]any)
		if !ok || len(items) != 3 {
			t.Fatalf("%s: %#v", field, body[field])
		}
		if wire == core.WireAPIResponses {
			assertJSON(t, items[1:], `[{"type":"function_call_output","call_id":"call-1","output":"<binary stdout: 75 bytes>\n[image:image/png]\n[video:video/mp4]"},{"role":"user","content":[{"type":"input_text","text":"[media returned by tool call call-1]"},{"type":"input_image","image_url":"data:image/png;base64,QUFBQQ==","detail":"auto"},{"type":"input_image","image_url":"data:video/mp4;base64,TVA0","detail":"auto"}]}]`)
		} else {
			assertJSON(t, items[1:], `[{"role":"tool","tool_call_id":"call-1","content":"<binary stdout: 75 bytes>\n[image:image/png]\n[video:video/mp4]"},{"role":"user","content":[{"type":"text","text":"[media returned by tool call call-1]"},{"type":"image_url","image_url":{"url":"data:image/png;base64,QUFBQQ==","detail":"auto"}},{"type":"image_url","image_url":{"url":"data:video/mp4;base64,TVA0","detail":"auto"}}]}]`)
		}
	}
}

func TestReasoningContentPolicy(t *testing.T) {
	items := []provider.InferenceItem{user("hello"), &provider.AssistantThinking{Text: "inspect "}, &provider.AssistantThinking{Text: "first"}, &provider.AssistantRedactedThinking{ModelID: "deepseek-v4-pro", Data: "opaque"}, toolUse("call-1", "a.ts"), toolResult("call-1", "contents")}
	for _, enabled := range []bool{false, true} {
		body := bodyOf(t.Context(), t, core.WireAPIChatCompletions, provider.VendorPolicy{PassBackReasoningContent: enabled}, requestWith(items...))
		messages, ok := body["messages"].([]any)
		if !ok || len(messages) != 3 {
			t.Fatalf("messages: %#v", body["messages"])
		}
		want := `{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.ts\"}"}}]}`
		if enabled {
			want = `{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.ts\"}"}}],"reasoning_content":"inspect first"}`
		}
		assertJSON(t, messages[1], want)
	}
	request := requestWith(user("hello"), &provider.AssistantThinking{Text: "first thought"}, toolUse("call-1", "a.ts"), toolResult("call-1", "a"), toolUse("call-2", "b.ts"), toolResult("call-2", "b"))
	body := bodyOf(t.Context(), t, core.WireAPIChatCompletions, provider.VendorPolicy{PassBackReasoningContent: true}, request)
	assertJSON(t, body["messages"], `[{"role":"user","content":"hello"},{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.ts\"}"}}],"reasoning_content":"first thought"},{"role":"tool","tool_call_id":"call-1","content":"a"},{"role":"assistant","content":null,"tool_calls":[{"id":"call-2","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"b.ts\"}"}}],"reasoning_content":""},{"role":"tool","tool_call_id":"call-2","content":"b"}]`)
	request = requestWith(user("first"), &provider.AssistantThinking{Text: "thought one"}, &provider.AssistantText{Text: "answer one"}, user("second"), &provider.AssistantThinking{Text: "orphaned"}, user("third"), &provider.AssistantText{Text: "answer three"})
	body = bodyOf(t.Context(), t, core.WireAPIChatCompletions, provider.VendorPolicy{PassBackReasoningContent: true}, request)
	assertJSON(t, body["messages"], `[{"role":"user","content":"first"},{"role":"assistant","content":"answer one","reasoning_content":"thought one"},{"role":"user","content":"second"},{"role":"user","content":"third"},{"role":"assistant","content":"answer three"}]`)
}
