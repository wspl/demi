package anthropicapi_test

import (
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"net/url"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/anthropicapi"
	"github.com/wspl/demi/go/provider/providertest"
)

func runtimeAt(t *testing.T, server *providertest.MockVendor) provider.ProviderRuntime {
	t.Helper()
	base, err := url.Parse(server.Server.URL + "/v1")
	if err != nil {
		t.Fatal(err)
	}
	key, err := provider.NewSecret("test-key")
	if err != nil {
		t.Fatal(err)
	}
	p, err := anthropicapi.New(anthropicapi.Config{APIKey: key, BaseURL: base}, providertest.FixedClock{})
	if err != nil {
		t.Fatal(err)
	}
	catalog, err := p.ListModels(t.Context())
	if err != nil || len(catalog.Models) != 5 || *catalog.DefaultModelID != "claude-opus-4-8" {
		t.Fatal(catalog, err)
	}
	runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: server.Server.Client()})
	if err != nil {
		t.Fatal(err)
	}
	return runtime
}
func eventsOf(t *testing.T, frames ...string) []provider.ProviderEvent {
	t.Helper()
	values := make([]jsontext.Value, len(frames))
	for i, frame := range frames {
		values[i] = jsontext.Value(frame)
	}
	server := providertest.NewMockVendor(t, providertest.MockResponse{Status: 200, Chunks: []string{providertest.SSEBody(values...)}})
	return providertest.Run(t, runtimeAt(t, server), providertest.InferenceRequest())
}
func TestMessagesStreamUsageThinkingAndToolInputs(t *testing.T) {
	events := eventsOf(t,
		`{"type":"message_start","message":{"usage":{"input_tokens":12,"cache_read_input_tokens":2,"cache_creation_input_tokens":1,"output_tokens":1}}}`,
		`{"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}`,
		`{"type":"ping"}`, `{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"plan"}}`,
		`{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}`,
		`{"type":"content_block_start","index":1,"content_block":{"type":"text","text":"hello"}}`,
		`{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t0","name":"read","input":{}}}`,
		`{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a.ts\"}"}}`,
		`{"type":"content_block_stop","index":2}`,
		`{"type":"content_block_start","index":3,"content_block":{"type":"redacted_thinking","data":"opaque"}}`,
		`{"type":"message_delta","usage":{"input_tokens":0,"output_tokens":5}}`, `{"type":"message_stop"}`, `bad`)
	want := []provider.ProviderEvent{provider.ThinkingStart{}, provider.ThinkingDelta{Text: "plan"}, provider.ThinkingSignature{Signature: "anthropic:sig"}, provider.TextDelta{Text: "hello"}, provider.ToolCall{ToolUseID: "t0", ToolName: "read", Input: jsontext.Value(`{"path":"a.ts"}`)}, provider.RedactedThinking{Data: "anthropic:opaque"}, provider.Response{Usage: core.TokenUsage{InputTokens: 12, OutputTokens: 5, CacheReadTokens: 2, CacheWriteTokens: 1}}}
	if !reflect.DeepEqual(events, want) {
		t.Fatalf("events: %#v", events)
	}
	for _, tc := range []struct{ input, delta, want string }{{`,"input":{"q":1}`, "", `{"q":1}`}, {"", "", `{}`}, {`,"input":{}`, `{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{"}}`, `"{"`}} {
		frames := []string{`{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t","name":"a"` + tc.input + `}}`}
		if tc.delta != "" {
			frames = append(frames, tc.delta)
		}
		frames = append(frames, `{"type":"content_block_stop","index":0}`)
		got := eventsOf(t, frames...)
		if len(got) != 2 || string(got[0].(provider.ToolCall).Input) != tc.want {
			t.Fatal(got)
		}
	}
	for _, reason := range []string{"end_turn", "tool_use", "max_tokens", "stop_sequence", "refusal", "pause_turn"} {
		got := eventsOf(t, fmt.Sprintf(`{"type":"message_delta","delta":{"stop_reason":%q},"usage":{"output_tokens":2}}`, reason))
		if !reflect.DeepEqual(got, []provider.ProviderEvent{provider.Response{Usage: core.TokenUsage{OutputTokens: 2}}}) {
			t.Fatal(got)
		}
	}
}
func TestMessagesMalformedEventsAndVendorFailures(t *testing.T) {
	for _, tc := range []struct {
		frame, field string
		code         provider.ErrorCode
	}{
		{`{"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}`, "index", ""},
		{`{"type":"content_block_start","index":0,"content_block":{"type":"text","text":42}}`, "text", ""},
		{`{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"","name":"a"}}`, "id", ""},
		{`{"type":"message_delta","usage":{"output_tokens":1.5}}`, "output_tokens", ""},
		{`{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}`, "Overloaded", provider.Overloaded},
		{`{"type":"error","error":{"type":"rate_limit_error","message":"Number of requests exceeded"}}`, "Number of requests", provider.RateLimit},
		{`{"type":"error","error":{"type":"invalid_request_error","message":42}}`, "Anthropic API stream error", "invalid_request_error"},
		{`{"type":"error","message":"stream closed"}`, "stream closed", "error"},
	} {
		events := eventsOf(t, tc.frame, `{"type":"message_stop"}`)
		if len(events) != 1 {
			t.Fatal(events)
		}
		failure, ok := events[0].(provider.FailureEvent)
		if !ok || failure.Failure.Code != tc.code || !strings.Contains(failure.Failure.Message, tc.field) || *failure.Failure.Diagnostics.Upstream != tc.frame {
			t.Fatalf("%s: %#v", tc.frame, events)
		}
	}
	got := eventsOf(t, `{"type":"new_event"}`, `{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use"}}`, `{"type":"content_block_delta","index":0,"delta":{"type":"citations_delta"}}`)
	if !reflect.DeepEqual(got, []provider.ProviderEvent{provider.Response{}}) {
		t.Fatal(got)
	}
}
func TestSessionCacheAndRequestPolicy(t *testing.T) {
	for _, tc := range []struct {
		system string
		cache  *provider.PromptCache
		marks  int
	}{{"system", &provider.PromptCache{AnsweredItems: 2}, 3}, {" ", &provider.PromptCache{AnsweredItems: 1}, 3}, {"system", &provider.PromptCache{}, 2}, {"system", nil, 0}} {
		server := providertest.NewMockVendor(t, providertest.MockResponse{Status: 200})
		runtime := runtimeAt(t, server)
		request := providertest.InferenceRequest()
		request.SystemPrompt = tc.system
		request.PromptCache = tc.cache
		request.Tools = []provider.ToolDefinition{{Name: "read", InputSchema: jsontext.Value(`{}`)}}
		request.Thinking = core.ThinkingConfigBudget{BudgetTokens: 99999}
		request.OutputLimit = new(uint32(4096))
		request.Items = append(request.Items, provider.AssistantThinking{Text: "plan", Signature: new("anthropic:sig")}, provider.AssistantText{Text: "Read all"})
		for i := range 12 {
			id := fmt.Sprintf("t%d", i)
			request.Items = append(request.Items, provider.ToolUse{ToolUseID: id, ToolName: "read", Input: jsontext.Value(`null`)}, provider.ToolResult{ToolUseID: id, Output: []provider.ResultPart{provider.TextPart{Text: "contents"}}})
		}
		request.Items = append(request.Items, provider.UserSteer{Content: []provider.UserPart{provider.TextPart{Text: "also check b.ts"}}})
		providertest.Run(t, runtime, request)
		sent := server.Requests()[0]
		if sent.URI != "/v1/messages" || sent.Headers.Get("X-Api-Key") != "test-key" || sent.Headers.Get("Anthropic-Version") != "2023-06-01" {
			t.Fatal(sent)
		}
		if strings.Count(string(sent.Body), `"cache_control"`) != tc.marks || strings.Count(string(sent.Body), `"ttl":"1h"`) != tc.marks {
			t.Fatal(string(sent.Body))
		}
		var body struct {
			Messages []struct {
				Role    string                      `json:"role"`
				Content []map[string]jsontext.Value `json:"content"`
			} `json:"messages"`
			MaxTokens uint32 `json:"max_tokens"`
			Thinking  struct {
				Budget uint32 `json:"budget_tokens"`
			} `json:"thinking"`
		}
		if err := json.Unmarshal(sent.Body, &body); err != nil {
			t.Fatal(err)
		}
		if len(body.Messages) != 25 || string(body.Messages[24].Content[1]["text"]) != `"also check b.ts"` || body.MaxTokens != 4096 || body.Thinking.Budget != 3072 {
			t.Fatal(string(sent.Body))
		}
		if tc.cache != nil && tc.cache.AnsweredItems > 0 {
			if body.Messages[0].Content[0]["cache_control"] == nil || body.Messages[1].Content[0]["cache_control"] != nil {
				t.Fatal("thinking received cache mark")
			}
		}
		ctx, cancel := context.WithCancel(t.Context())
		cancel()
		for event := range runtime.Run(ctx, request) {
			t.Fatal(event)
		}
		if len(server.Requests()) != 1 {
			t.Fatal("cancelled request sent")
		}
	}
}

func TestMessagesReplayMediaAndOmissions(t *testing.T) {
	server := providertest.NewMockVendor(t, providertest.MockResponse{})
	request := providertest.InferenceRequest()
	image := provider.MediaBytes{Data: core.NewB64Bytes([]byte("image")), MediaType: "image/png"}
	request.Items = []provider.InferenceItem{
		provider.UserMessage{Content: []provider.UserPart{provider.ImagePart{Medium: image}, provider.ImagePart{Medium: provider.MediaURL{URL: "https://example.test/image"}}, provider.DocumentPart{Bytes: provider.MediaBytes{Data: core.NewB64Bytes([]byte("pdf")), MediaType: "application/pdf"}, FileName: "file.pdf"}, provider.VideoPart{Medium: provider.MediaURL{URL: "https://example.test/video"}}}},
		provider.AssistantThinking{Text: "own", Signature: new("anthropic:signature")},
		provider.AssistantThinking{Text: "foreign", Signature: new("openai:signature")},
		provider.AssistantThinking{Text: "old", Signature: new("anthropic:signature"), KeptPastSummary: true},
		provider.AssistantRedactedThinking{Data: "anthropic:opaque"},
		provider.AssistantRedactedThinking{Data: "anthropic:old", KeptPastSummary: true},
		provider.AssistantText{Text: "answer"},
		provider.ToolUse{ToolUseID: "call", ToolName: "read", Input: jsontext.Value(`null`)},
		provider.ToolResult{ToolUseID: "call", IsError: true, Output: []provider.ResultPart{provider.ResultImage{Bytes: image}, provider.ResultVideo{Bytes: provider.MediaBytes{Data: core.NewB64Bytes(nil), MediaType: "video/mp4"}}}},
	}
	request.SystemPrompt = " \n"
	providertest.Run(t, runtimeAt(t, server), request)
	var body struct {
		Messages []struct {
			Role    string                      `json:"role"`
			Content []map[string]jsontext.Value `json:"content"`
		} `json:"messages"`
		System jsontext.Value `json:"system"`
		Tools  jsontext.Value `json:"tools"`
		Tier   jsontext.Value `json:"service_tier"`
	}
	if err := json.Unmarshal(server.Requests()[0].Body, &body); err != nil {
		t.Fatal(err)
	}
	if body.System != nil || body.Tools != nil || body.Tier != nil || len(body.Messages) != 3 {
		t.Fatalf("body %s", server.Requests()[0].Body)
	}
	user := body.Messages[0].Content
	assistant := body.Messages[1].Content
	if len(user) != 4 || string(user[0]["type"]) != `"image"` || string(user[1]["text"]) != `"[image:https://example.test/image]"` || string(user[2]["title"]) != `"file.pdf"` || string(user[3]["text"]) != `"[video]"` {
		t.Fatalf("user %#v", user)
	}
	if len(assistant) != 4 || string(assistant[0]["signature"]) != `"signature"` || string(assistant[1]["data"]) != `"opaque"` || string(assistant[3]["input"]) != `{}` {
		t.Fatalf("assistant %#v", assistant)
	}
	result := body.Messages[2].Content[0]
	if string(result["is_error"]) != "true" || !strings.Contains(string(result["content"]), "[video:video/mp4]") {
		t.Fatalf("tool result %#v", result)
	}
}
