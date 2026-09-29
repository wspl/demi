package openaiapi_test

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"net/http"
	"net/url"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/openaiapi"
	"github.com/wspl/demi/go/provider/providertest"
)

func at(t *testing.T, vendor *providertest.MockVendor, wire core.WireAPI, policy openaiapi.VendorPolicy) *openaiapi.Provider {
	t.Helper()
	base, err := url.Parse(vendor.Server.URL + "/v1")
	if err != nil {
		t.Fatal(err)
	}
	key, err := provider.NewSecret("test-key")
	if err != nil {
		t.Fatal(err)
	}
	p, err := openaiapi.New(openaiapi.Config{APIKey: key, BaseURL: base, Wire: wire, Policy: policy}, providertest.FixedClock{})
	if err != nil {
		t.Fatal(err)
	}
	return p
}
func bodyOf(t *testing.T, wire core.WireAPI, policy openaiapi.VendorPolicy, request provider.InferenceRequest) map[string]jsontext.Value {
	t.Helper()
	server := providertest.NewMockVendor(t, providertest.MockResponse{Status: 200})
	p := at(t, server, wire, policy)
	runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: server.Server.Client()})
	if err != nil {
		t.Fatal(err)
	}
	events := providertest.Run(t, runtime, request)
	if !reflect.DeepEqual(events, []provider.ProviderEvent{provider.Response{}}) {
		t.Fatal(events)
	}
	var body map[string]jsontext.Value
	if err := json.Unmarshal(server.Requests()[0].Body, &body); err != nil {
		t.Fatal(err)
	}
	return body
}
func TestBothWiresEndpointsLimitsCatalogAndRefusals(t *testing.T) {
	for _, wire := range []core.WireAPI{core.WireAPIResponses, core.WireAPIChatCompletions} {
		t.Run(string(wire), func(t *testing.T) {
			server := providertest.NewMockVendor(t, providertest.MockResponse{Status: 429, Headers: http.Header{"Retry-After": {"1.25"}}, Chunks: []string{"quota exceeded"}})
			p := at(t, server, wire, openaiapi.VendorPolicy{})
			catalog, err := p.ListModels(t.Context())
			if err != nil || len(catalog.Models) != 4 || *catalog.DefaultModelID != "gpt-5.5" || len(server.Requests()) != 0 {
				t.Fatal(catalog, err)
			}
			if _, ok := p.AuthStatus(t.Context()).(core.AuthStateAuthenticated); !ok {
				t.Fatal("auth state")
			}
			runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: server.Server.Client()})
			if err != nil {
				t.Fatal(err)
			}
			limits := runtime.RequestLimits(core.Model{})
			if *limits.BodyBytes != 512000000 || *limits.Images != 1500 {
				t.Fatal(limits)
			}
			events := providertest.Run(t, runtime, providertest.InferenceRequest())
			if len(events) != 1 {
				t.Fatal(events)
			}
			failure := events[0].(provider.FailureEvent).Failure
			if failure.Code != provider.RateLimit || *failure.RetryAfter != 1250 || provider.ReadHTTPFailureRecord(*failure.Diagnostics).Body != "quota exceeded" {
				t.Fatal(failure)
			}
			sent := server.Requests()[0]
			endpoint := "/v1/responses"
			if wire == core.WireAPIChatCompletions {
				endpoint = "/v1/chat/completions"
			}
			if sent.URI != endpoint || sent.Headers.Get("Authorization") != "Bearer test-key" || sent.Headers.Get("Accept") != "text/event-stream" || sent.Headers.Get("Content-Type") != "application/json" {
				t.Fatal(sent)
			}
			for _, limits := range []struct {
				limit, cap *uint32
				want       string
			}{{nil, nil, ""}, {new(uint32(500)), nil, "500"}, {nil, new(uint32(200)), "200"}, {new(uint32(500)), new(uint32(200)), "200"}} {
				request := providertest.InferenceRequest()
				request.OutputLimit = limits.limit
				request.OutputCap = limits.cap
				body := bodyOf(t, wire, openaiapi.VendorPolicy{}, request)
				field := "max_output_tokens"
				if wire == core.WireAPIChatCompletions {
					field = "max_completion_tokens"
				}
				if string(body[field]) != limits.want {
					t.Fatal(body)
				}
			}
		})
	}
}
func TestTranscriptPoliciesAndThinking(t *testing.T) {
	request := providertest.InferenceRequest()
	request.SystemPrompt = "system"
	request.ServiceTierID = new("priority")
	request.Tools = []provider.ToolDefinition{{Name: "read", Description: "Read file", InputSchema: jsontext.Value(`{"type":"object"}`)}}
	request.Thinking = core.ThinkingConfigEffort{Effort: "high", Summary: new(core.ThinkingSummaryOff)}
	request.Items = append(request.Items, provider.AssistantThinking{Text: "inspect "}, provider.AssistantText{Text: "Use tool"}, provider.AssistantThinking{Text: "first"}, provider.ToolUse{ToolUseID: "call|item", ToolName: "read", Input: jsontext.Value(`{"path":"a.ts"}`)}, provider.ToolResult{ToolUseID: "call|item", Output: []provider.ResultPart{provider.TextPart{Text: "contents"}}})
	responses := bodyOf(t, core.WireAPIResponses, openaiapi.VendorPolicy{ReplayAssistantStatus: true}, request)
	if string(responses["reasoning"]) != `{"effort":"high"}` || string(responses["service_tier"]) != `"priority"` || string(responses["parallel_tool_calls"]) != "true" {
		t.Fatal(responses)
	}
	var input []map[string]jsontext.Value
	if err := json.Unmarshal(responses["input"], &input); err != nil {
		t.Fatal(err)
	}
	if len(input) != 4 || string(input[1]["status"]) != `"completed"` || string(input[2]["call_id"]) != `"call"` || string(input[2]["id"]) != `"item"` {
		t.Fatal(input)
	}
	for _, passBack := range []bool{false, true} {
		body := bodyOf(t, core.WireAPIChatCompletions, openaiapi.VendorPolicy{PassBackReasoningContent: passBack}, request)
		var messages []map[string]jsontext.Value
		if err := json.Unmarshal(body["messages"], &messages); err != nil {
			t.Fatal(err)
		}
		if len(messages) != 4 || string(messages[2]["content"]) != `"Use tool"` || string(body["reasoning_effort"]) != `"high"` {
			t.Fatal(messages, body)
		}
		want := ""
		if passBack {
			want = `"inspect first"`
		}
		if string(messages[2]["reasoning_content"]) != want {
			t.Fatal(messages)
		}
	}
	for _, thinking := range []core.ThinkingConfig{nil, core.ThinkingConfigBudget{BudgetTokens: 2048}, core.ThinkingConfigDisabled{}} {
		request := providertest.InferenceRequest()
		request.SystemPrompt = " \n"
		request.Thinking = thinking
		for _, wire := range []core.WireAPI{core.WireAPIResponses, core.WireAPIChatCompletions} {
			body := bodyOf(t, wire, openaiapi.VendorPolicy{}, request)
			for _, field := range []string{"instructions", "tools", "tool_choice", "reasoning", "reasoning_effort", "service_tier"} {
				if _, ok := body[field]; ok {
					t.Fatal(field, body)
				}
			}
		}
	}
}
func TestResponsesMediaAndOwnedReasoningReplay(t *testing.T) {
	request := providertest.InferenceRequest()
	image := provider.MediaBytes{Data: core.NewB64Bytes([]byte("image")), MediaType: "image/png"}
	video := provider.MediaBytes{Data: core.NewB64Bytes([]byte("video")), MediaType: "video/mp4"}
	request.Items = []provider.InferenceItem{
		provider.UserMessage{Content: []provider.UserPart{provider.ImagePart{Medium: image}, provider.ImagePart{Medium: provider.MediaURL{URL: "https://example.test/image"}}, provider.VideoPart{Medium: video}, provider.DocumentPart{Bytes: provider.MediaBytes{Data: core.NewB64Bytes([]byte("pdf")), MediaType: "application/pdf"}, FileName: "file.pdf"}}},
		provider.AssistantThinking{Text: "reason", Signature: new(`openai:{"id":"r","type":"reasoning","summary":[{"type":"summary_text","text":"reason"}],"encrypted_content":"cipher","private":"extra"}`)},
		provider.AssistantThinking{Text: "foreign", Signature: new(`codex:{"id":"foreign","type":"reasoning"}`)},
		provider.AssistantText{Text: "answer"},
		provider.ToolUse{ToolUseID: "call", ToolName: "read", Input: jsontext.Value(`null`)},
		provider.ToolResult{ToolUseID: "call", Output: []provider.ResultPart{provider.TextPart{Text: "result"}, provider.ResultImage{Bytes: image}, provider.ResultVideo{Bytes: video}}},
	}
	body := bodyOf(t, core.WireAPIResponses, openaiapi.VendorPolicy{}, request)
	var input []map[string]jsontext.Value
	if err := json.Unmarshal(body["input"], &input); err != nil {
		t.Fatal(err)
	}
	if len(input) != 6 {
		t.Fatalf("input %s", body["input"])
	}
	if string(input[1]["encrypted_content"]) != `"cipher"` || input[1]["private"] != nil || input[2]["status"] != nil || string(input[3]["arguments"]) != `"{}"` {
		t.Fatalf("replay %s", body["input"])
	}
	var content []map[string]jsontext.Value
	if err := json.Unmarshal(input[0]["content"], &content); err != nil {
		t.Fatal(err)
	}
	if len(content) != 4 || string(content[0]["image_url"]) != `"data:image/png;base64,aW1hZ2U="` || string(content[1]["image_url"]) != `"https://example.test/image"` || string(content[3]["type"]) != `"input_file"` {
		t.Fatalf("media %s", input[0]["content"])
	}
	if string(input[5]["role"]) != `"user"` || !strings.Contains(string(input[5]["content"]), "data:image/png;base64,") {
		t.Fatalf("tool media %s", input[5]["content"])
	}
}
