package codex_test

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
)

func sent(t *testing.T, r provider.InferenceRequest) providertest.RecordedRequest {
	t.Helper()
	v, _, p := setup(t)
	v.RespondAt(responses, completed())
	run(t.Context(), t, p, v.Client(), r)
	return v.Requests()[0]
}

func TestTranscriptBody(t *testing.T) {
	r := providertest.InferenceRequest()
	r.ModelID = "gpt-5.4"
	r.SystemPrompt = "system"
	signature := `codex:{"type":"reasoning","id":"rs_1","encrypted_content":"enc",` +
		`"summary":[{"text":"summary"}],"status":"completed"}`
	r.Items = []provider.InferenceItem{
		&provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: "hello"}}},
		&provider.UserSteer{Content: []provider.UserPart{&provider.TextPart{Text: "steer this turn"}}},
		&provider.AssistantThinking{ModelID: r.ModelID, Text: "private", Signature: &signature},
		&provider.AssistantText{ModelID: r.ModelID, Text: "visible"},
		&provider.ToolUse{
			ModelID:   r.ModelID,
			ToolUseID: "call_1|fc_1",
			ToolName:  "shell_exec",
			Input:     json.RawMessage(`{"script":"pwd"}`),
		},
		&provider.ToolResult{ToolUseID: "call_1|fc_1", Output: []provider.ResultPart{&provider.TextPart{Text: "/tmp"}}},
	}
	r.Tools = []provider.ToolDefinition{
		{Name: "shell_exec", Description: "Execute shell", InputSchema: json.RawMessage(`{"type":"object"}`)},
	}
	r.Thinking = &types.EffortConfig{Effort: "medium"}
	body := jsonObject(t, sent(t, r).Body)
	expected := `{"model":"gpt-5.4","instructions":"system",` +
		`"input":[{"role":"user","content":[{"type":"input_text",` +
		`"text":"hello"}]},{"role":"user","content":[{"type":"input_text",` +
		`"text":"steer this turn"}]},{"type":"reasoning","id":"rs_1",` +
		`"encrypted_content":"enc","summary":[{"text":"summary"}],` +
		`"status":"completed"},{"type":"message","role":"assistant","id":"msg_` +
		provider.ShortHash(
			"3:gpt-5.4:visible",
		) +
		`","status":"completed","content":[{"type":"output_text",` +
		`"text":"visible","annotations":[]}]},{"type":"function_call",` +
		`"id":"fc_1","call_id":"call_1","name":"shell_exec",` +
		`"arguments":"{\"script\":\"pwd\"}"},` +
		`{"type":"function_call_output","call_id":"call_1","output":"/` +
		`tmp"}],"tools":[{"type":"function","name":"shell_exec",` +
		`"description":"Execute shell","parameters":{"type":"object"},` +
		`"strict":null}],"tool_choice":"auto","parallel_tool_calls":true,` +
		`"store":false,"stream":true,` +
		`"include":["reasoning.encrypted_content"],` +
		`"prompt_cache_key":"session-1","text":{"verbosity":"low"},` +
		`"reasoning":{"effort":"medium","summary":"auto"}}`
	equal(t, body, jsonObject(t, []byte(expected)))
}

func TestForeignThinkingAndInlineToolImages(t *testing.T) {
	r := providertest.InferenceRequest()
	signature := `openai:{"type":"reasoning","encrypted_content":"enc"}`
	r.SystemPrompt = ""
	r.Tools = nil
	r.Items = []provider.InferenceItem{
		&provider.AssistantThinking{
			Text: "unsigned",
		},
		&provider.AssistantThinking{Text: "openai", Signature: &signature},
		&provider.ToolResult{
			ToolUseID: "call_1|fc_1",
			Output: []provider.ResultPart{
				&provider.TextPart{Text: "see image"},
				&provider.ResultImage{Bytes: provider.MediaBytes{Data: []byte{1, 2, 3}, MediaType: "image/png"}},
				&provider.ResultVideo{Bytes: provider.MediaBytes{Data: []byte{4}, MediaType: "video/mp4"}},
			},
		},
	}
	body := jsonObject(t, sent(t, r).Body)
	expected := jsonObject(
		t,
		[]byte(
			`{"input":[{"type":"function_call_output","call_id":"call_1",`+
				`"output":[{"type":"input_text","text":"see image"},`+
				`{"type":"input_image","image_url":"data:image/png;base64,AQID",`+
				`"detail":"auto"}]}]}`,
		),
	)
	equal(t, body["input"], expected["input"])
	equal(t, body["instructions"], "")
	equal(t, body["tools"], []any{})
}

func TestTiersAndEfforts(t *testing.T) {
	r := providertest.InferenceRequest()
	r.Thinking = nil
	body := jsonObject(t, sent(t, r).Body)
	if _, ok := body["reasoning"]; ok {
		t.Fatal("unexpected reasoning")
	}
	if _, ok := body["service_tier"]; ok {
		t.Fatal("unexpected tier")
	}
	tier := "priority"
	r.ServiceTierID = &tier
	equal(t, jsonObject(t, sent(t, r).Body)["service_tier"], tier)
	r.ServiceTierID = nil
	for _, effort := range []string{"low", "xhigh", "max", "ultra", "high"} {
		summary := types.ThinkingSummary("off")
		r.Thinking = &types.EffortConfig{Effort: effort}
		if effort == "high" {
			r.Thinking = &types.EffortConfig{Effort: effort, Summary: &summary}
		}
		equal(t, jsonObject(t, sent(t, r).Body)["reasoning"], map[string]any{"effort": effort, "summary": "auto"})
	}
}

func TestInferenceHeaders(t *testing.T) {
	r := providertest.InferenceRequest()
	for _, session := range []string{"session-1", "chat-" + strings.Repeat("x", 80)} {
		r.SessionID = session
		request := sent(t, r)
		key := provider.PromptCacheKey(session)
		if len(key) > 64 {
			t.Fatalf("cache key too long: %q", key)
		}
		for name, want := range map[string]string{
			"Authorization": "Bearer " +
				freshToken(t),
			"Chatgpt-Account-Id":  "acct-1",
			"Openai-Beta":         "responses=experimental",
			"Session-Id":          key,
			"Thread-Id":           key,
			"X-Client-Request-Id": "request-1",
			"Accept":              "text/event-stream",
		} {
			equal(t, request.Header(name), want)
		}
		equal(t, jsonObject(t, request.Body)["prompt_cache_key"], key)
		if !strings.HasPrefix(request.Header("User-Agent"), "demi-codex-provider/") {
			t.Fatal("missing user agent")
		}
		equal(t, request.Headers.Values("X-Openai-Fedramp"), []string(nil))
	}
}

func TestFedrampHeader(t *testing.T) {
	v := providertest.StartVendor(t)
	access := providertest.JWT(
		t,
		map[string]any{
			"exp":                         1789743600,
			"https://api.openai.com/auth": map[string]any{"chatgpt_account_is_fedramp": true},
		},
	)
	pool := poolWith(t, document(t, access, "refresh-1", now))
	p := configured(t, v, pool, nil)
	v.RespondAt(responses, completed())
	run(t.Context(), t, p, v.Client(), providertest.InferenceRequest())
	equal(t, v.Requests()[0].Header("X-Openai-Fedramp"), "true")
}
