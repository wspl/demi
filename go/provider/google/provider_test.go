package google_test

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"net/url"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/google"
	"github.com/wspl/demi/go/provider/providertest"
)

func runtimeAt(t *testing.T, server *providertest.MockVendor) provider.ProviderRuntime {
	t.Helper()
	base, err := url.Parse(server.Server.URL + "/v1beta?ignored=yes")
	if err != nil {
		t.Fatal(err)
	}
	key, err := provider.NewSecret("test-key")
	if err != nil {
		t.Fatal(err)
	}
	p, err := google.New(google.Config{APIKey: key, BaseURL: base}, providertest.FixedClock{})
	if err != nil {
		t.Fatal(err)
	}
	catalog, err := p.ListModels(t.Context())
	if err != nil || len(catalog.Models) != 4 || *catalog.DefaultModelID != "gemini-3.6-flash" || !(*catalog.Models[0].SupportsVideo) {
		t.Fatal(catalog, err)
	}
	runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: server.Server.Client()})
	if err != nil {
		t.Fatal(err)
	}
	return runtime
}
func eventsOf(t *testing.T, frames ...jsontext.Value) []provider.ProviderEvent {
	t.Helper()
	server := providertest.NewMockVendor(t, providertest.MockResponse{Status: 200, Chunks: []string{providertest.SSEBody(frames...)}})
	return providertest.Run(t, runtimeAt(t, server), providertest.InferenceRequest())
}
func TestGeminiStreamThinkingSignaturesCallsAndUsage(t *testing.T) {
	events := eventsOf(t,
		jsontext.Value(`{"candidates":[{"content":{"parts":[{"text":"plan","thought":true}]}}]}`),
		jsontext.Value(`{"candidates":[{"content":{"parts":[{"functionCall":{"name":"shell","args":{"command":"ls"},"id":"c1"},"thoughtSignature":"sig-1"}]}}]}`),
		jsontext.Value(`{"candidates":[{"content":{"parts":[{"functionCall":{"name":"look","id":"c2"},"thoughtSignature":"sig-2"},{"thought":true,"text":"more"},{"text":"","thoughtSignature":"sig-3"},{"text":"done"}]}}]}`),
		jsontext.Value(`{"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":4,"thoughtsTokenCount":20,"cachedContentTokenCount":3}}`))
	want := []provider.ProviderEvent{provider.ThinkingStart{}, provider.ThinkingDelta{Text: "plan"}, provider.ThinkingSignature{Signature: "google:sig-1"}, provider.ToolCall{ToolUseID: "c1", ToolName: "shell", Input: jsontext.Value(`{"command":"ls"}`)}, provider.ThinkingStart{}, provider.ThinkingSignature{Signature: "google:sig-2"}, provider.ToolCall{ToolUseID: "c2", ToolName: "look", Input: jsontext.Value(`{}`)}, provider.ThinkingStart{}, provider.ThinkingDelta{Text: "more"}, provider.ThinkingSignature{Signature: "google:sig-3"}, provider.TextDelta{Text: "done"}, provider.Response{Usage: core.TokenUsage{InputTokens: 7, OutputTokens: 24, CacheReadTokens: 3}}}
	if !reflect.DeepEqual(events, want) {
		t.Fatalf("events: %#v", events)
	}
	ids := map[string]bool{}
	for range 2 {
		events := eventsOf(t, jsontext.Value(`{"candidates":[{"content":{"parts":[{"functionCall":{"name":"shell"}},{"functionCall":{"name":"shell"}}]}}]}`))
		for _, event := range events {
			if call, ok := event.(provider.ToolCall); ok {
				if !strings.HasPrefix(call.ToolUseID, "shell_") || ids[call.ToolUseID] {
					t.Fatal(call.ToolUseID)
				}
				ids[call.ToolUseID] = true
			}
		}
	}
	if len(ids) != 4 {
		t.Fatal(ids)
	}
}
func TestGeminiMalformedChunksAndFailures(t *testing.T) {
	for _, tc := range []struct {
		frame, field string
		code         provider.ErrorCode
	}{
		{`{"candidates":{}}`, "candidates", ""},
		{`{"usageMetadata":{"promptTokenCount":"10"}}`, "promptTokenCount", ""},
		{`{"candidates":[{"content":{"parts":[{"functionCall":{"name":""}}]}}]}`, "name", ""},
		{`{"error":{"status":"RESOURCE_EXHAUSTED","message":"quota exceeded"}}`, "quota exceeded", provider.RateLimit},
	} {
		events := eventsOf(t, jsontext.Value(tc.frame), jsontext.Value(`bad`))
		if len(events) != 1 {
			t.Fatal(events)
		}
		failure, ok := events[0].(provider.FailureEvent)
		if !ok || failure.Failure.Code != tc.code || !strings.Contains(failure.Failure.Message, tc.field) || *failure.Failure.Diagnostics.Upstream != tc.frame {
			t.Fatalf("%s: %#v", tc.frame, events)
		}
	}
	server := providertest.NewMockVendor(t, providertest.MockResponse{Status: 429, Chunks: []string{"exhausted"}})
	failure := providertest.Run(t, runtimeAt(t, server), providertest.InferenceRequest())[0].(provider.FailureEvent).Failure
	if failure.Code != provider.RateLimit || provider.ReadHTTPFailureRecord(*failure.Diagnostics).Body != "exhausted" {
		t.Fatal(failure)
	}
}
func TestGeminiRequestsSchemasMediaSignaturesAndBudgets(t *testing.T) {
	for _, tc := range []struct {
		thinking core.ThinkingConfig
		budget   string
		limit    *uint32
		max      uint32
	}{{nil, "", nil, 32000}, {core.ThinkingConfigDisabled{}, "0", new(uint32(100)), 100}, {core.ThinkingConfigBudget{BudgetTokens: 2048}, "2048", nil, 32000}, {core.ThinkingConfigEffort{Effort: "low"}, "4096", nil, 32000}, {core.ThinkingConfigEffort{Effort: "high"}, "32768", nil, 32000}, {core.ThinkingConfigEffort{Effort: "xhigh"}, "65536", nil, 32000}, {core.ThinkingConfigEffort{Effort: "max"}, "98304", nil, 32000}, {core.ThinkingConfigAdaptive{Effort: "unknown"}, "16384", nil, 32000}} {
		server := providertest.NewMockVendor(t, providertest.MockResponse{Status: 200})
		request := providertest.InferenceRequest()
		request.ModelID = "model/escaped"
		request.SystemPrompt = "system"
		request.Thinking = tc.thinking
		request.OutputLimit = tc.limit
		request.Tools = []provider.ToolDefinition{{Name: "read", InputSchema: jsontext.Value(`{"type":"object","additionalProperties":false,"properties":{"x":{"type":"array","items":{"type":"string","minLength":1}}},"anyOf":[{"type":"object","$ref":"gone"}]}`)}}
		media := provider.MediaBytes{MediaType: "video/mp4", Data: core.NewB64Bytes([]byte("MP4"))}
		request.Items = []provider.InferenceItem{
			provider.UserMessage{Content: []provider.UserPart{provider.VideoPart{Medium: media}, provider.ImagePart{Medium: provider.MediaURL{URL: "https://example.invalid/a.png"}}}},
			provider.AssistantThinking{Signature: new("google:signed")}, provider.ToolUse{ToolUseID: "c1", ToolName: "read", Input: jsontext.Value(`null`)},
			provider.ToolResult{ToolUseID: "c1", Output: []provider.ResultPart{provider.TextPart{Text: "result"}, provider.ResultVideo{Bytes: media}}},
			provider.AssistantThinking{Signature: new("other:sig")}, provider.ToolUse{ToolUseID: "c2", ToolName: "read", Input: jsontext.Value(`{}`)}, provider.ToolResult{ToolUseID: "c2", Output: []provider.ResultPart{provider.ResultVideo{Bytes: media}}},
		}
		runtime := runtimeAt(t, server)
		providertest.Run(t, runtime, request)
		sent := server.Requests()[0]
		if sent.URI != "/v1beta/models/model%2Fescaped:streamGenerateContent?alt=sse" || sent.Headers.Get("X-Goog-Api-Key") != "test-key" {
			t.Fatal(sent.URI, sent.Headers)
		}
		var body struct {
			Contents []struct {
				Role  string                      `json:"role"`
				Parts []map[string]jsontext.Value `json:"parts"`
			} `json:"contents"`
			Config struct {
				Max      uint32                    `json:"maxOutputTokens"`
				Thinking map[string]jsontext.Value `json:"thinkingConfig"`
			} `json:"generationConfig"`
		}
		if err := json.Unmarshal(sent.Body, &body); err != nil {
			t.Fatal(err)
		}
		if body.Config.Max != tc.max || string(body.Config.Thinking["thinkingBudget"]) != tc.budget {
			t.Fatal(string(sent.Body))
		}
		if len(body.Contents) != 5 || string(body.Contents[1].Parts[0]["thoughtSignature"]) != `"signed"` || body.Contents[2].Parts[0]["functionResponse"] == nil || body.Contents[2].Parts[1]["inlineData"] == nil || string(body.Contents[3].Parts[0]["text"]) != `"[called read with {}]"` || string(body.Contents[4].Parts[0]["text"]) != `"[read returned] [video/mp4]"` {
			t.Fatal(string(sent.Body))
		}
		for _, removed := range []string{"additionalProperties", "minLength", "$ref"} {
			if strings.Contains(string(sent.Body), removed) {
				t.Fatal("schema keyword retained", removed)
			}
		}
		limits := runtime.RequestLimits(core.Model{})
		if *limits.BodyBytes != 20000000 || *limits.Images != 3600 {
			t.Fatal(limits)
		}
	}
}
