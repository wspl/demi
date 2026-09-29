package codex_test

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"strings"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/codex"
	"github.com/wspl/demi/go/provider/providertest"
)

// Cost: one loopback request; checks Codex-specific Responses replay policy.
func TestCodexTranscriptReasoningAndInlineToolImages(t *testing.T) {
	vendor := providertest.NewMockVendor(t, providertest.MockResponse{})
	p, pool := fixture(t, vendor.Server.URL, codex.TransportSSE)
	token, err := provider.NewSecret(providertest.JWT(jsontext.Value(`{"https://api.openai.com/auth":{"chatgpt_account_is_fedramp":true}}`)))
	if err != nil {
		t.Fatal(err)
	}
	document, err := (codex.SecretDocument{AccessToken: token, RefreshToken: token, IDToken: token, AccountID: "account", LastRefresh: core.UnixEpoch}).JSON()
	if err != nil {
		t.Fatal(err)
	}
	if err := pool.Write(t.Context(), provider.AccountMeta{ID: "one", Label: "account"}, string(document)); err != nil {
		t.Fatal(err)
	}
	runtime, err := p.Runtime(provider.RuntimeEnv{HTTP: vendor.Server.Client()})
	if err != nil {
		t.Fatal(err)
	}
	request := providertest.InferenceRequest()
	request.SystemPrompt = "system"
	request.ServiceTierID = new("priority")
	request.Thinking = core.ThinkingConfigEffort{Effort: "high", Summary: new(core.ThinkingSummaryOff)}
	request.Tools = []provider.ToolDefinition{{Name: "read", InputSchema: jsontext.Value(`{"type":"object"}`)}}
	request.Items = append(request.Items,
		provider.AssistantThinking{Text: "reason", Signature: new(`codex:{"type":"reasoning","id":"r","summary":[{"type":"summary_text","text":"reason"}],"encrypted_content":"cipher","extra":{"kept":true}}`)},
		provider.AssistantThinking{Text: "foreign", Signature: new(`openai:{"type":"reasoning","id":"foreign"}`)},
		provider.AssistantText{Text: "answer"},
		provider.ToolUse{ToolUseID: "call|item", ToolName: "read", Input: jsontext.Value(`null`)},
		provider.ToolResult{ToolUseID: "call|item", Output: []provider.ResultPart{provider.TextPart{Text: "result"}, provider.ResultImage{Bytes: provider.MediaBytes{Data: core.NewB64Bytes([]byte("image")), MediaType: "image/png"}}, provider.ResultVideo{Bytes: provider.MediaBytes{Data: core.NewB64Bytes([]byte("video")), MediaType: "video/mp4"}}}},
	)
	providertest.Run(t, runtime, request)
	if vendor.Requests()[0].Headers.Get("X-Openai-Fedramp") != "true" {
		t.Fatalf("fedramp header: %v", vendor.Requests()[0].Headers)
	}
	var body struct {
		Input     []map[string]jsontext.Value `json:"input"`
		Tools     []map[string]jsontext.Value `json:"tools"`
		Reasoning map[string]string           `json:"reasoning"`
		Tier      string                      `json:"service_tier"`
	}
	if err := json.Unmarshal(vendor.Requests()[0].Body, &body); err != nil {
		t.Fatal(err)
	}
	if len(body.Input) != 5 || body.Tier != "priority" || body.Reasoning["effort"] != "high" || body.Reasoning["summary"] != "auto" || string(body.Tools[0]["strict"]) != "null" {
		t.Fatalf("request %s", vendor.Requests()[0].Body)
	}
	if string(body.Input[1]["extra"]) != `{"kept":true}` || string(body.Input[1]["encrypted_content"]) != `"cipher"` || string(body.Input[2]["status"]) != `"completed"` || !strings.HasPrefix(string(body.Input[2]["id"]), `"msg_`) {
		t.Fatalf("replay %#v", body.Input)
	}
	if string(body.Input[3]["call_id"]) != `"call"` || string(body.Input[3]["id"]) != `"item"` || string(body.Input[3]["arguments"]) != `"{}"` {
		t.Fatalf("tool %#v", body.Input[3])
	}
	output := string(body.Input[4]["output"])
	if !strings.Contains(output, "data:image/png;base64,aW1hZ2U=") || !strings.Contains(output, `"text":"result"`) || strings.Contains(output, "video") {
		t.Fatalf("tool media %s", output)
	}
}
