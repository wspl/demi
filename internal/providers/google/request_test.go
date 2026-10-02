package google_test

import (
	"encoding/json"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestOutputLimits(t *testing.T) {
	for _, tc := range []struct {
		name       string
		limit, cap *uint32
		want       float64
	}{
		{"model", new(uint32(8000)), nil, 8000},
		{"default", nil, nil, 32000},
		{"cap", new(uint32(8000)), new(uint32(1024)), 1024},
	} {
		t.Run(tc.name, func(t *testing.T) {
			request := providertest.InferenceRequest()
			request.OutputLimit, request.OutputCap = tc.limit, tc.cap
			got := bodyOf(t, request)["generationConfig"].(map[string]any)["maxOutputTokens"]
			if got != tc.want {
				t.Fatalf("got %v want %v", got, tc.want)
			}
		})
	}
}

func TestSystemToolsAndThinking(t *testing.T) {
	request := providertest.InferenceRequest()
	request.Items = []provider.InferenceItem{&provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: "hi"}}}}
	request.SystemPrompt = "you are a shell"
	request.Tools = []provider.ToolDefinition{{Name: "shell_exec", Description: "run", InputSchema: []byte(`{"type":"object"}`)}}
	request.Thinking = &core.EffortConfig{Effort: "high"}
	body := bodyOf(t, request)
	equalJSON(t, body["systemInstruction"], `{"parts":[{"text":"you are a shell"}]}`)
	equalJSON(t, body["tools"], `[{"functionDeclarations":[{"name":"shell_exec","description":"run","parameters":{"type":"object"}}]}]`)
	equalJSON(t, body["contents"], `[{"role":"user","parts":[{"text":"hi"}]}]`)
	equalJSON(t, body["generationConfig"].(map[string]any)["thinkingConfig"], `{"includeThoughts":true,"thinkingBudget":32768}`)
	for _, tc := range []struct {
		name   string
		config core.ThinkingConfig
		want   string
	}{
		{"budget", &core.BudgetConfig{BudgetTokens: 2048}, `{"includeThoughts":true,"thinkingBudget":2048}`},
		{"unknown adaptive", &core.AdaptiveConfig{Effort: "unheard-of"}, `{"includeThoughts":true,"thinkingBudget":16384}`},
		{"disabled", &core.DisabledConfig{}, `{"includeThoughts":false,"thinkingBudget":0}`},
		{"default", nil, `{"includeThoughts":true}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			request.Thinking = tc.config
			equalJSON(t, bodyOf(t, request)["generationConfig"].(map[string]any)["thinkingConfig"], tc.want)
		})
	}
	blank := bodyOf(t, providertest.InferenceRequest())
	if _, ok := blank["systemInstruction"]; ok {
		t.Fatal("blank instruction emitted")
	}
	if _, ok := blank["tools"]; ok {
		t.Fatal("empty tools emitted")
	}
}

func TestGeminiToolSchemas(t *testing.T) {
	request := providertest.InferenceRequest()
	request.Tools = []provider.ToolDefinition{{Name: "shell_exec", Description: "run", InputSchema: []byte(`{
 "type":"object","$schema":"https://json-schema.org/draft/2020-12/schema","additionalProperties":false,"required":["script"],
 "properties":{"script":{"type":"string","minLength":1,"description":"the script"},"tags":{"type":"array","items":{"type":"string","pattern":"^[a-z]+$"}},"mode":{"anyOf":[{"type":"string","const":"fast"},{"type":"null"}]}}}`)}}
	equalJSON(t, bodyOf(t, request)["tools"], `[{"functionDeclarations":[{"name":"shell_exec","description":"run","parameters":{
 "type":"object","required":["script"],"properties":{"script":{"type":"string","description":"the script"},"tags":{"type":"array","items":{"type":"string"}},"mode":{"anyOf":[{"type":"string"},{"type":"null"}]}}}}]}]`)
}

func TestSignedCallReplay(t *testing.T) {
	request := providertest.InferenceRequest()
	signature := "google:sig-abc"
	request.Items = []provider.InferenceItem{
		&provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: "list files"}}},
		&provider.AssistantThinking{Signature: &signature},
		&provider.ToolUse{ToolUseID: "call-1", ToolName: "shell_exec", Input: []byte(`{"command":"ls"}`)},
		&provider.ToolResult{ToolUseID: "call-1", Output: []provider.ResultPart{&provider.TextPart{Text: "a.md"}}},
	}
	equalJSON(t, bodyOf(t, request)["contents"], `[
 {"role":"user","parts":[{"text":"list files"}]},
 {"role":"model","parts":[{"functionCall":{"name":"shell_exec","args":{"command":"ls"},"id":"call-1"},"thoughtSignature":"sig-abc"}]},
 {"role":"user","parts":[{"functionResponse":{"name":"shell_exec","id":"call-1","response":{"output":"a.md"}}}]}]`)
}

func TestUnsignedCallReplay(t *testing.T) {
	for _, signature := range []*string{new(`{"type":"reasoning","encrypted_content":"…"}`), new("google:"), nil} {
		request := providertest.InferenceRequest()
		request.Items = []provider.InferenceItem{
			&provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: "list files"}}},
			&provider.AssistantThinking{Signature: signature},
			&provider.ToolUse{ToolUseID: "call-9", ToolName: "shell_exec", Input: []byte(`{"command":"ls"}`)},
			&provider.ToolResult{ToolUseID: "call-9", Output: []provider.ResultPart{&provider.TextPart{Text: "a.md"}, &provider.ResultImage{Bytes: provider.MediaBytes{Data: []byte("PNG"), MediaType: "image/png"}}}},
		}
		equalJSON(t, bodyOf(t, request)["contents"], `[
  {"role":"user","parts":[{"text":"list files"}]},
  {"role":"model","parts":[{"text":"[called shell_exec with {\"command\":\"ls\"}]"}]},
  {"role":"user","parts":[{"text":"[shell_exec returned] a.md\n[image/png]"}]}]`)
	}
}

func TestInlineVideoAndToolMedia(t *testing.T) {
	request := providertest.InferenceRequest()
	request.Items = []provider.InferenceItem{
		&provider.UserMessage{Content: []provider.UserPart{
			&provider.TextPart{Text: "watch this"},
			&provider.VideoPart{Medium: &provider.MediaBytes{Data: []byte("VID"), MediaType: "video/mp4"}},
			&provider.ImagePart{Medium: &provider.MediaURL{URL: "https://example.com/a.png"}},
		}},
		&provider.AssistantThinking{Signature: new("google:sig-2")},
		&provider.ToolUse{ToolUseID: "call-2", ToolName: "look", Input: []byte(`null`)},
		&provider.ToolResult{ToolUseID: "call-2", Output: []provider.ResultPart{
			&provider.TextPart{Text: "rendered"},
			&provider.ResultImage{Bytes: provider.MediaBytes{Data: []byte("PNG"), MediaType: "image/png"}},
		}},
	}
	equalJSON(t, bodyOf(t, request)["contents"], `[
 {"role":"user","parts":[{"text":"watch this"},{"inlineData":{"mimeType":"video/mp4","data":"VklE"}},{"fileData":{"fileUri":"https://example.com/a.png"}}]},
 {"role":"model","parts":[{"functionCall":{"name":"look","args":{},"id":"call-2"},"thoughtSignature":"sig-2"}]},
 {"role":"user","parts":[{"functionResponse":{"name":"look","id":"call-2","response":{"output":"rendered"}}},{"inlineData":{"mimeType":"image/png","data":"UE5H"}}]}]`)
}

func TestReplayRoleAndSignatureBoundaries(t *testing.T) {
	request := providertest.InferenceRequest()
	request.Items = []provider.InferenceItem{
		&provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: "one"}}},
		&provider.UserSteer{Content: []provider.UserPart{&provider.TextPart{Text: "two"}}},
		&provider.AssistantThinking{Signature: new("google:kept")},
		&provider.AssistantRedactedThinking{Data: "ignored"},
		&provider.ToolUse{ToolUseID: "a", ToolName: "look", Input: []byte(`null`)},
		&provider.ToolUse{ToolUseID: "b", ToolName: "look", Input: []byte(`"bad arguments"`)},
		&provider.AssistantThinking{Signature: new("google:cleared")},
		&provider.AssistantText{Text: "answer"},
		&provider.ToolUse{ToolUseID: "c", ToolName: "look", Input: []byte(`{}`)},
		&provider.ToolResult{ToolUseID: "orphan", Output: []provider.ResultPart{&provider.TextPart{Text: "orphan result"}}},
	}
	equalJSON(t, bodyOf(t, request)["contents"], `[
 {"role":"user","parts":[{"text":"one"},{"text":"two"}]},
 {"role":"model","parts":[{"functionCall":{"name":"look","args":{},"id":"a"},"thoughtSignature":"kept"},{"text":"[called look with bad arguments]"},{"text":"answer"},{"text":"[called look with {}]"}]},
 {"role":"user","parts":[{"functionResponse":{"name":"tool","id":"orphan","response":{"output":"orphan result"}}}]}]`)
}

// TestGeminiSchemaWireOrder pins Rust's insertion order after schema reduction,
// including objects in keywords whose values are retained without reduction.
func TestGeminiSchemaWireOrder(t *testing.T) {
	request := providertest.InferenceRequest()
	request.Tools = []provider.ToolDefinition{{Name: "ordered", Description: "order", InputSchema: []byte(`{
 "type":"object","additionalProperties":false,"required":["zeta"],
 "properties":{
  "zeta":{"description":"first","type":"array","items":{"title":"item","type":"object","properties":{"z":{"type":"string","minLength":1},"a":{"description":"second","type":"number"}}}},
  "alpha":{"anyOf":[{"description":"choice","type":"string","const":"drop"},{"type":"null"}],"default":{"z":1,"a":{"y":2,"b":3}},"enum":[{"z":4,"a":5}]}
 },"default":{"zeta":{"z":6,"a":7},"alpha":null}
 }`)}}
	v := providertest.StartVendor(t)
	v.Respond(providertest.EventStream(""))
	events := providertest.Run(t.Context(), t, runtimeAt(t, v, "/v1beta"), request)
	if len(events) != 1 {
		t.Fatalf("unexpected events: %#v", events)
	}
	sent, err := provider.DecodeUntagged[struct {
		Tools []struct {
			Declarations []struct {
				Parameters json.RawMessage `json:"parameters"`
			} `json:"functionDeclarations"`
		} `json:"tools"`
	}](string(v.Requests()[0].Body))
	if err != nil {
		t.Fatal(err)
	}
	// Rust iterates serde_json::Map in read order and inserts each retained key
	// in that same order, recursively for properties, items and anyOf.
	want := `{"type":"object","required":["zeta"],"properties":{"zeta":{"description":"first","type":"array","items":{"title":"item","type":"object","properties":{"z":{"type":"string"},"a":{"description":"second","type":"number"}}}},"alpha":{"anyOf":[{"description":"choice","type":"string"},{"type":"null"}],"default":{"z":1,"a":{"y":2,"b":3}},"enum":[{"z":4,"a":5}]}},"default":{"zeta":{"z":6,"a":7},"alpha":null}}`
	got := string(sent.Tools[0].Declarations[0].Parameters)
	if got != want {
		t.Fatalf("schema wire bytes:\n got %s\nwant %s", got, want)
	}
}
