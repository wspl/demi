package anthropicapi_test

import (
	"fmt"
	"reflect"
	"sort"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
)

func TestMessagesEndpointHeaders(t *testing.T) {
	v := providertest.StartVendor(t)
	for _, base := range []string{"/v1", "/v1/", "/v1/messages"} {
		v.Respond(recorded(`{"type":"message_stop"}`))
		r, err := providerAt(t, v, base, provider.VendorPolicy{}).Runtime(provider.RuntimeEnv{HTTP: v.Client()})
		if err != nil {
			t.Fatal(err)
		}
		equalEvents(
			t,
			providertest.Run(t.Context(), t, r, providertest.InferenceRequest()),
			[]provider.Event{&provider.Response{}},
		)
		if err := r.Close(t.Context()); err != nil {
			t.Fatal(err)
		}
	}
	for _, request := range v.Requests() {
		if request.Method != "POST" || request.URI != "/v1/messages" {
			t.Fatal(request)
		}
		for header, want := range map[string]string{
			"x-api-key":         "sk-ant-test",
			"anthropic-version": "2023-06-01",
			"content-type":      "application/json",
			"accept":            "text/event-stream",
		} {
			if got := request.Header(header); got != want {
				t.Fatalf("%s: %q", header, got)
			}
		}
	}
}

func TestGroupedTurnsToolsSystemTier(t *testing.T) {
	request := providertest.InferenceRequest()
	request.ModelID = "claude-test"
	request.SystemPrompt = "system instructions"
	tier := "standard_only"
	limit := uint32(8192)
	request.ServiceTierID = &tier
	request.OutputLimit = &limit
	request.Thinking = &types.BudgetConfig{BudgetTokens: 1024}
	request.Items = append(request.Items,
		&provider.AssistantText{Text: "Use tool"},
		&provider.ToolUse{ToolUseID: "toolu-1", ToolName: "read_file", Input: []byte(`{"path":"a.ts"}`)},
		&provider.ToolResult{ToolUseID: "toolu-1", Output: []provider.ResultPart{&provider.TextPart{Text: "contents"}}},
		&provider.UserSteer{Content: []provider.UserPart{&provider.TextPart{Text: "also check b.ts"}}},
		&provider.ToolUse{ToolUseID: "toolu-2", ToolName: "read_file", Input: []byte(`null`)},
		&provider.ToolResult{ToolUseID: "toolu-2", IsError: true},
	)
	request.Tools = []provider.ToolDefinition{
		{
			Name:        "read_file",
			Description: "Read a file",
			InputSchema: []byte(`{"type":"object","properties":{"path":{"type":"string"}}}`),
		},
	}
	equalJSON(
		t,
		sentBody(t, request, provider.VendorPolicy{}),
		"{\n \"model\":\"claude-test\",\"messages\":[\n {\"role\":\"user\","+
			"\"content\":[{\"type\":\"text\",\"text\":\"hello\"}]},\n "+
			`{"role":"assistant","content":[{"type":"text","text":"Use tool"},`+
			`{"type":"tool_use","id":"toolu-1","name":"read_file",`+
			"\"input\":{\"path\":\"a.ts\"}}]},\n {\"role\":\"user\","+
			`"content":[{"type":"tool_result","tool_use_id":"toolu-1",`+
			`"content":[{"type":"text","text":"contents"}]},{"type":"text",`+
			"\"text\":\"also check b.ts\"}]},\n {\"role\":\"assistant\","+
			`"content":[{"type":"tool_use","id":"toolu-2","name":"read_file",`+
			"\"input\":{}}]},\n {\"role\":\"user\","+
			`"content":[{"type":"tool_result","tool_use_id":"toolu-2",`+
			"\"content\":[],\"is_error\":true}]}\n ],\"max_tokens\":8192,"+
			`"stream":true,"system":[{"type":"text","text":"system `+
			"instructions\"}],\n \"tools\":[{\"name\":\"read_file\","+
			`"description":"Read a file","input_schema":{"type":"object",`+
			"\"properties\":{\"path\":{\"type\":\"string\"}}}}],\n "+
			`"thinking":{"type":"enabled","budget_tokens":1024},`+
			`"service_tier":"standard_only"}`,
	)
}

// cacheMarks finds the vendor cache boundaries and verifies their lifetime.
func cacheMarks(t *testing.T, value any, path string) []string {
	t.Helper()
	out := []string{}
	switch value := value.(type) {
	case map[string]any:
		for key, v := range value {
			if key == "cache_control" {
				equalJSON(t, v, `{"type":"ephemeral","ttl":"1h"}`)
				out = append(out, path)
			} else {
				out = append(out, cacheMarks(t, v, path+"/"+key)...)
			}
		}
	case []any:
		for i, v := range value {
			out = append(out, cacheMarks(t, v, fmt.Sprintf("%s/%d", path, i))...)
		}
	}
	sort.Strings(out)
	return out
}

func TestSessionCacheMarks(t *testing.T) {
	request := providertest.InferenceRequest()
	sig := "anthropic:sig-1"
	request.Items = append(
		request.Items,
		&provider.AssistantThinking{Text: "plan", Signature: &sig},
		&provider.AssistantText{Text: "Reading them all"},
	)
	for call := 0; call < 12; call++ {
		id := fmt.Sprintf("toolu-%d", call)
		request.Items = append(
			request.Items,
			&provider.ToolUse{
				ToolUseID: id,
				ToolName:  "read_file",
				Input:     []byte(fmt.Sprintf(`{"path":"%d.ts"}`, call)),
			},
			&provider.ToolResult{ToolUseID: id, Output: []provider.ResultPart{&provider.TextPart{Text: "contents"}}},
		)
	}
	request.Items = append(
		request.Items,
		&provider.UserSteer{Content: []provider.UserPart{&provider.TextPart{Text: "also check b.ts"}}},
	)
	request.Tools = []provider.ToolDefinition{
		{Name: "read_file", Description: "Read a file", InputSchema: []byte(`{}`)},
	}
	for _, c := range []struct {
		name, system string
		answered     int
		want         []string
	}{
		{"system", "system", 1, []string{"/messages/0/content/0", "/messages/24/content/1", "/system/0"}},
		{"tools", " ", 1, []string{"/messages/0/content/0", "/messages/24/content/1", "/tools/0"}},
		{"thinking", "system", 2, []string{"/messages/0/content/0", "/messages/24/content/1", "/system/0"}},
		{"first", "system", 0, []string{"/messages/24/content/1", "/system/0"}},
		{"off", "system", -1, []string{}},
	} {
		t.Run(c.name, func(t *testing.T) {
			request.SystemPrompt = c.system
			request.PromptCache.AnsweredItems = nil
			if c.answered >= 0 {
				request.PromptCache.AnsweredItems = &c.answered
			}
			body := sentBody(t, request, provider.VendorPolicy{})
			got := cacheMarks(t, body, "")
			if !reflect.DeepEqual(got, c.want) {
				t.Fatalf("marks %v; want %v", got, c.want)
			}
			messages := body["messages"].([]any)
			content := messages[24].(map[string]any)["content"].([]any)
			if content[1].(map[string]any)["text"] != "also check b.ts" {
				t.Fatal(content)
			}
		})
	}
}

func TestBlankOptionalFieldsOmitted(t *testing.T) {
	request := providertest.InferenceRequest()
	request.SystemPrompt = " \n\t"
	body := sentBody(t, request, provider.VendorPolicy{})
	for _, key := range []string{"system", "tools", "thinking", "output_config", "service_tier"} {
		if _, ok := body[key]; ok {
			t.Fatal(key, body)
		}
	}
	if body["max_tokens"] != float64(32000) {
		t.Fatal(body)
	}
}

func TestReusedRuntimeOutputLimits(t *testing.T) {
	v := providertest.StartVendor(t)
	r := testRuntime(t, v, provider.VendorPolicy{})
	for i, c := range []struct {
		model      string
		limit, cap uint32
		want       float64
	}{
		{"a", 8000, 0, 8000}, {"b", 32000, 0, 32000}, {"c", 0, 0, 32000}, {"d", 8000, 1024, 1024}, {"e", 0, 1024, 1024},
	} {
		v.Respond(recorded(`{"type":"message_stop"}`))
		request := providertest.InferenceRequest()
		request.ModelID = c.model
		if c.limit != 0 {
			request.OutputLimit = &c.limit
		}
		if c.cap != 0 {
			request.OutputCap = &c.cap
		}
		equalEvents(t, providertest.Run(t.Context(), t, r, request), []provider.Event{&provider.Response{}})
		sent := v.Requests()
		if len(sent) != i+1 {
			t.Fatalf("requests: got %d, want %d", len(sent), i+1)
		}
		body := sent[len(sent)-1].JSON(t).(map[string]any)
		if body["model"] != c.model || body["max_tokens"] != c.want {
			t.Fatal(body)
		}
	}
}

func TestThinkingConfiguration(t *testing.T) {
	off := types.ThinkingSummary("off")
	for _, c := range []struct {
		name             string
		config           types.ThinkingConfig
		limit            uint32
		policy           provider.VendorPolicy
		thinking, output string
	}{
		{
			"ceiling",
			&types.BudgetConfig{
				BudgetTokens: 999999,
			},
			8192,
			provider.VendorPolicy{},
			`{"type":"enabled","budget_tokens":7168}`,
			`null`,
		},
		{
			"minimum",
			&types.BudgetConfig{
				BudgetTokens: 100,
			},
			0,
			provider.VendorPolicy{},
			`{"type":"enabled","budget_tokens":1024}`,
			`null`,
		},
		{
			"effort",
			&types.EffortConfig{
				Effort: "high",
			},
			0,
			provider.VendorPolicy{},
			`{"type":"adaptive","display":"summarized"}`,
			`{"effort":"high"}`,
		},
		{
			"omitted",
			&types.EffortConfig{
				Effort:  "low",
				Summary: &off,
			},
			0,
			provider.VendorPolicy{},
			`{"type":"adaptive","display":"omitted"}`,
			`{"effort":"low"}`,
		},
		{
			"adaptive",
			&types.AdaptiveConfig{
				Effort: "max",
			},
			0,
			provider.VendorPolicy{},
			`{"type":"adaptive","display":"summarized"}`,
			`{"effort":"max"}`,
		},
		{"disabled", &types.DisabledConfig{}, 0, provider.VendorPolicy{}, `null`, `null`},
		{"absent", nil, 0, provider.VendorPolicy{}, `null`, `null`},
		{
			"budget-high",
			&types.EffortConfig{
				Effort: "high",
			},
			128000,
			provider.VendorPolicy{
				EffortAsBudget: true,
			},
			`{"type":"enabled","budget_tokens":32768}`,
			`null`,
		},
		{
			"budget-max",
			&types.AdaptiveConfig{
				Effort: "max",
			},
			8192,
			provider.VendorPolicy{
				EffortAsBudget: true,
			},
			`{"type":"enabled","budget_tokens":7168}`,
			`null`,
		},
		{
			"budget-unknown",
			&types.EffortConfig{
				Effort: "minimal",
			},
			128000,
			provider.VendorPolicy{
				EffortAsBudget: true,
			},
			`{"type":"enabled","budget_tokens":16384}`,
			`null`,
		},
	} {
		t.Run(c.name, func(t *testing.T) {
			request := providertest.InferenceRequest()
			request.Thinking = c.config
			if c.limit != 0 {
				request.OutputLimit = &c.limit
			}
			body := sentBody(t, request, c.policy)
			equalJSON(t, body["thinking"], c.thinking)
			equalJSON(t, body["output_config"], c.output)
			if c.policy.EffortAsBudget {
				if _, present := body["output_config"]; present {
					t.Fatal("budget-only vendor received output_config")
				}
			}
		})
	}
}

func TestThinkingReplayOwnershipAndSummary(t *testing.T) {
	request := providertest.InferenceRequest()
	kept, own, other := "anthropic:sig-0", "anthropic:sig-1", "google:sig-2"
	request.Items = append(request.Items,
		&provider.AssistantThinking{Text: "kept", Signature: &kept, KeptPastSummary: true},
		&provider.AssistantRedactedThinking{Data: "anthropic:kept-opaque", KeptPastSummary: true},
		&provider.AssistantText{Text: "hello"},
		&provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: "hi"}}},
		&provider.AssistantThinking{Text: "plan", Signature: &own},
		&provider.AssistantThinking{Text: "theirs", Signature: &other},
		&provider.AssistantThinking{Text: "unsigned"},
		&provider.AssistantRedactedThinking{Data: "anthropic:opaque"},
		&provider.AssistantRedactedThinking{Data: "opaque-elsewhere"},
		&provider.ToolUse{ToolUseID: "toolu-1", ToolName: "ls", Input: []byte(`{}`)},
	)
	messages := sentBody(t, request, provider.VendorPolicy{})["messages"].([]any)
	equalJSON(t, messages[1], `{"role":"assistant","content":[{"type":"text","text":"hello"}]}`)
	equalJSON(
		t,
		messages[3],
		`{"role":"assistant","content":[{"type":"thinking",`+
			`"thinking":"plan","signature":"sig-1"},`+
			`{"type":"redacted_thinking","data":"opaque"},{"type":"tool_use",`+
			`"id":"toolu-1","name":"ls","input":{}}]}`,
	)
}

func TestInlineMediaAndPlaceholders(t *testing.T) {
	png := provider.MediaBytes{Data: []byte{0x89, 'P', 'N', 'G'}, MediaType: "image/png"}
	request := providertest.InferenceRequest()
	request.Items = []provider.InferenceItem{
		&provider.UserMessage{Content: []provider.UserPart{
			&provider.ImagePart{Medium: &png},
			&provider.ImagePart{Medium: &provider.MediaURL{URL: "https://example.com/a.png"}},
			&provider.DocumentPart{
				Bytes:    provider.MediaBytes{Data: []byte("%PDF"), MediaType: "application/pdf"},
				FileName: "spec.pdf",
			},
			&provider.VideoPart{Medium: &provider.MediaBytes{Data: png.Data, MediaType: "video/mp4"}},
		}},
		&provider.ToolUse{ToolUseID: "toolu-1", ToolName: "shot", Input: []byte(`{}`)},
		&provider.ToolResult{
			ToolUseID: "toolu-1",
			Output: []provider.ResultPart{
				&provider.ResultImage{Bytes: png},
				&provider.ResultVideo{
					Bytes: provider.MediaBytes{Data: []byte{0x1a, 0x45, 0xdf, 0xa3}, MediaType: "video/webm"},
				},
			},
		},
	}
	messages := sentBody(t, request, provider.VendorPolicy{})["messages"].([]any)
	equalJSON(
		t,
		messages[0].(map[string]any)["content"],
		"[\n {\"type\":\"image\",\"source\":{\"type\":\"base64\","+
			"\"media_type\":\"image/png\",\"data\":\"iVBORw==\"}},\n "+
			"{\"type\":\"text\",\"text\":\"[image:https://example.com/a.png]\"},\n "+
			`{"type":"document","source":{"type":"base64",`+
			`"media_type":"application/pdf","data":"JVBERg=="},`+
			"\"title\":\"spec.pdf\"},\n {\"type\":\"text\",\"text\":\"[video]\"}]",
	)
	result := messages[2].(map[string]any)["content"].([]any)[0].(map[string]any)
	equalJSON(
		t,
		result["content"],
		`[{"type":"image","source":{"type":"base64","media_type":"image/`+
			`png","data":"iVBORw=="}},{"type":"text","text":"[video:video/webm]"}]`,
	)
}

// TestRequestWireOrder pins the bytes used by the vendor's prompt cache. Each
// case costs one local scripted request; the expected field order is the
// declaration order of the request types in request_types.go.
func TestRequestWireOrder(t *testing.T) {
	const messages = `"model":"model-1","messages":[{"role":"user",` +
		`"content":[{"type":"text","text":"hello <>&\u2028\u2029"},` +
		`{"type":"image","source":{"type":"base64","media_type":"image/` +
		`png","data":"iVBORw=="}},{"type":"document",` +
		`"source":{"type":"base64","media_type":"application/pdf",` +
		`"data":"JVBERg=="},"title":"spec.pdf",` +
		`"cache_control":{"type":"ephemeral","ttl":"1h"}}]},` +
		`{"role":"assistant","content":[{"type":"text","text":"Reading"},` +
		`{"type":"thinking","thinking":"plan","signature":"sig"},` +
		`{"type":"redacted_thinking","data":"opaque"},{"type":"tool_use",` +
		`"id":"call-1","name":"read","input":{"z":1,"a":{"y":2,"b":3}}},` +
		`{"type":"tool_use","id":"call-2","name":"read","input":{}}]},` +
		`{"role":"user","content":[{"type":"tool_result",` +
		`"tool_use_id":"call-1","content":[{"type":"text","text":"result"}` +
		`,{"type":"image","source":{"type":"base64","media_type":"image/` +
		`png","data":"iVBORw=="}}],"is_error":true},{"type":"tool_result",` +
		`"tool_use_id":"call-2","content":[],` +
		`"cache_control":{"type":"ephemeral","ttl":"1h"}}]}],` +
		`"max_tokens":8192,"stream":true,`
	const tool = `"tools":[{"name":"read","description":"Read a file",` +
		`"input_schema":{"type":"object",` +
		`"properties":{"z":{"type":"number"},"a":{"type":"object",` +
		`"properties":{"y":{"type":"number"},"b":{"type":"number"}}}}}`
	const system = `"system":[{"type":"text","text":"system",` +
		`"cache_control":{"type":"ephemeral","ttl":"1h"}}],`
	// The request writes these Unicode separators literally, not as JSON escapes.
	prefix := strings.ReplaceAll(strings.ReplaceAll(messages, `\u2028`, "\u2028"), `\u2029`, "\u2029")
	for _, c := range []struct {
		name, system string
		thinking     types.ThinkingConfig
		nullInput    bool
		want         string
	}{
		{
			"adaptive",
			"system",
			&types.EffortConfig{
				Effort: "high",
			},
			false,
			"{" +
				prefix +
				system +
				tool +
				`}],"thinking":{"type":"adaptive","display":"summarized"},` +
				`"output_config":{"effort":"high"},"service_tier":"standard_only"}`,
		},
		{
			"budget",
			"system",
			&types.BudgetConfig{
				BudgetTokens: 1024,
			},
			true,
			"{" +
				prefix +
				system +
				tool +
				`}],"thinking":{"type":"enabled","budget_tokens":1024},` +
				`"service_tier":"standard_only"}`,
		},
		{
			"tool-cache",
			"",
			&types.AdaptiveConfig{
				Effort: "max",
			},
			false,
			"{" +
				prefix +
				tool +
				`,"cache_control":{"type":"ephemeral","ttl":"1h"}}],` +
				`"thinking":{"type":"adaptive","display":"summarized"},` +
				`"output_config":{"effort":"max"},"service_tier":"standard_only"}`,
		},
	} {
		t.Run(c.name, func(t *testing.T) {
			png := provider.MediaBytes{Data: []byte{0x89, 'P', 'N', 'G'}, MediaType: "image/png"}
			signature := "anthropic:sig"
			answered := 1
			limit := uint32(8192)
			tier := "standard_only"
			emptyCall := &provider.ToolUse{ToolUseID: "call-2", ToolName: "read"}
			if c.nullInput {
				emptyCall.Input = []byte(`null`)
			}
			request := providertest.InferenceRequest()
			request.SystemPrompt = c.system
			request.OutputLimit = &limit
			request.ServiceTierID = &tier
			request.Thinking = c.thinking
			request.PromptCache.AnsweredItems = &answered
			request.Tools = []provider.ToolDefinition{
				{
					Name:        "read",
					Description: "Read a file",
					InputSchema: []byte(
						`{"type":"object","properties":{"z":{"type":"number"},` +
							`"a":{"type":"object","properties":{"y":{"type":"number"},` +
							`"b":{"type":"number"}}}}}`,
					),
				},
			}
			request.Items = []provider.InferenceItem{
				&provider.UserMessage{
					Content: []provider.UserPart{
						&provider.TextPart{Text: "hello <>&\u2028\u2029"},
						&provider.ImagePart{Medium: &png},
						&provider.DocumentPart{
							Bytes:    provider.MediaBytes{Data: []byte("%PDF"), MediaType: "application/pdf"},
							FileName: "spec.pdf",
						},
					},
				},
				&provider.AssistantText{Text: "Reading"},
				&provider.AssistantThinking{Text: "plan", Signature: &signature},
				&provider.AssistantRedactedThinking{Data: "anthropic:opaque"},
				&provider.ToolUse{ToolUseID: "call-1", ToolName: "read", Input: []byte(`{"z":1,"a":{"y":2,"b":3}}`)},
				emptyCall,
				&provider.ToolResult{
					ToolUseID: "call-1",
					Output: []provider.ResultPart{
						&provider.TextPart{Text: "result"},
						&provider.ResultImage{Bytes: png},
					},
					IsError: true,
				},
				&provider.ToolResult{ToolUseID: "call-2"},
			}
			v := providertest.StartVendor(t)
			v.Respond(recorded(`{"type":"message_stop"}`))
			equalEvents(
				t,
				providertest.Run(t.Context(), t, testRuntime(t, v, provider.VendorPolicy{}), request),
				[]provider.Event{&provider.Response{}},
			)
			if got := string(v.Requests()[0].Body); got != c.want {
				t.Fatalf("request bytes:\n%s\nwant:\n%s", got, c.want)
			}
		})
	}
}
