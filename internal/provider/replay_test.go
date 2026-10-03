package provider_test

import (
	"context"
	"errors"
	"io"
	"net/http"
	"net/url"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
)

func TestResponsesReplayDialectsPreserveHistory(t *testing.T) {
	signature := `openai:{"type":"reasoning","id":"rs_1",` +
		`"summary":[{"text":"thought","z":2,"a":1}],` +
		`"encrypted_content":"enc","z":2,"a":1}`
	image := provider.MediaBytes{Data: []byte{1, 2}, MediaType: "image/png"}
	video := provider.MediaBytes{Data: []byte{3}, MediaType: "video/mp4"}
	items := []provider.InferenceItem{
		&provider.UserMessage{
			Content: []provider.UserPart{
				&provider.TextPart{Text: "<hello> &\u2028\u2029"},
				&provider.ImagePart{Medium: &image},
				&provider.VideoPart{Medium: &provider.MediaURL{URL: "https://video.invalid/x"}},
				&provider.DocumentPart{
					FileName: "a.pdf",
					Bytes:    provider.MediaBytes{Data: []byte{4}, MediaType: "application/pdf"},
				},
			},
		},
		&provider.AssistantThinking{Signature: &signature, KeptPastSummary: true},
		&provider.AssistantText{ModelID: "m", Text: "answer"},
		&provider.ToolUse{ToolUseID: "call|item", ToolName: "read", Input: []byte(`{"z":2,"a":"<>&\u2028\u2029"}`)},
		&provider.ToolResult{
			ToolUseID: "call|item",
			Output: []provider.ResultPart{
				&provider.TextPart{Text: "result"},
				&provider.ResultImage{Bytes: image},
				&provider.ResultVideo{Bytes: video},
			},
		},
	}
	dialect := provider.ResponsesDialect{
		SignatureTag: "openai:",
		Assistant:    provider.AssistantIdentified,
		Reasoning:    provider.ReasoningWhole,
		ToolMedia:    provider.ToolMediaFollowUp,
	}
	input, err := provider.ResponsesInput(items, dialect)
	if err != nil {
		t.Fatal(err)
	}
	if len(input) != 6 {
		t.Fatalf("%#v", input)
	}
	whole := encoded(t, input)
	if !strings.Contains(whole, `"z":2,"a":1`) || strings.Contains(whole, `\u2028`) ||
		strings.Contains(whole, `\u003c`) {
		t.Fatalf("replay rewrote order or escaping: %s", whole)
	}
	reason, ok := input[1].(*provider.ReasoningItem)
	if !ok {
		t.Fatalf("%#v", input[1])
	}
	requireEqual(t, reason.Text(), "thought")
	requireEqual(
		t,
		encoded(t, input[3]),
		`{"type":"function_call","id":"item","call_id":"call",`+
			`"name":"read","arguments":"{\"z\":2,\"a\":\"<>&`+"\u2028\u2029"+`\"}"}`,
	)
	requireEqual(
		t,
		jsonValue(t, encoded(t, input[4])),
		jsonValue(
			t,
			`{"type":"function_call_output","call_id":"call",`+
				`"output":"result\n[image:image/png]\n[video:video/mp4]"}`,
		),
	)
	// Each dialect changes only the fields its endpoint accepts.
	dialect.Assistant = provider.AssistantMinimal
	dialect.Reasoning = provider.ReasoningReplayable
	dialect.ToolMedia = provider.ToolMediaInline
	minimal, err := provider.ResponsesInput(items, dialect)
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, len(minimal), 5)
	requireEqual(
		t,
		encoded(t, minimal[2]),
		`{"type":"message","role":"assistant",`+
			`"content":[{"type":"output_text","text":"answer","annotations":[]}]}`,
	)
	requireEqual(
		t,
		encoded(t, minimal[1]),
		`{"type":"reasoning","id":"rs_1","summary":[{"text":"thought",`+
			`"z":2,"a":1}],"encrypted_content":"enc"}`,
	)
	requireEqual(
		t,
		encoded(t, minimal[4]),
		`{"type":"function_call_output","call_id":"call",`+
			`"output":[{"type":"input_text","text":"result"},`+
			`{"type":"input_image","image_url":"data:image/png;base64,AQI=",`+
			`"detail":"auto"}]}`,
	)
	dialect.SignatureTag = "codex:"
	foreign, err := provider.ResponsesInput(items, dialect)
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, len(foreign), 4)
	// Appending input must leave every earlier serialized item byte-identical.
	extended, err := provider.ResponsesInput(
		append(items, &provider.UserSteer{Content: []provider.UserPart{&provider.TextPart{Text: "more"}}}),
		dialect,
	)
	if err != nil {
		t.Fatal(err)
	}
	for i := range foreign {
		requireEqual(t, encoded(t, extended[i]), encoded(t, foreign[i]))
	}
}

func TestChatReplayCoalescesReasoningAndToolMedia(t *testing.T) {
	image := provider.MediaBytes{Data: []byte{1}, MediaType: "image/png"}
	items := []provider.InferenceItem{
		&provider.AssistantThinking{
			Text: "think ",
		},
		&provider.AssistantText{Text: "one"},
		&provider.AssistantThinking{Text: "more"},
		&provider.AssistantText{Text: "two"},
		&provider.ToolUse{ToolUseID: "id", ToolName: "tool", Input: []byte(`"invalid {"`)},
		&provider.ToolResult{
			ToolUseID: "id",
			Output:    []provider.ResultPart{&provider.TextPart{Text: "ok"}, &provider.ResultImage{Bytes: image}},
		},
		&provider.AssistantThinking{
			Text: "discarded",
		},
		&provider.UserMessage{
			Content: []provider.UserPart{&provider.TextPart{Text: "a"}, &provider.TextPart{Text: "b"}},
		},
		&provider.ToolUse{ToolUseID: "next", ToolName: "second", Input: []byte(`null`)},
	}
	messages, err := provider.ChatMessages(
		"system",
		items,
		provider.ChatDialect{ReasoningContent: true, Media: provider.ChatNative},
	)
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, len(messages), 6)
	requireEqual(
		t,
		encoded(t, messages[1]),
		`{"role":"assistant","content":"onetwo","tool_calls":[{"id":"id",`+
			`"type":"function","function":{"name":"tool","arguments":"invalid `+
			`{"}}],"reasoning_content":"think more"}`,
	)
	requireEqual(
		t,
		encoded(t, messages[3]),
		`{"role":"user","content":[{"type":"text","text":"[media returned `+
			`by tool call id]"},{"type":"image_url",`+
			`"image_url":{"url":"data:image/png;base64,AQ==","detail":"auto"}}]}`,
	)
	requireEqual(t, encoded(t, messages[4]), `{"role":"user","content":"a\nb"}`)
	requireEqual(
		t,
		encoded(t, messages[5]),
		`{"role":"assistant","content":null,"tool_calls":[{"id":"next",`+
			`"type":"function","function":{"name":"second","arguments":"{}"}}]`+
			`,"reasoning_content":""}`,
	)
	messages, err = provider.ChatMessages(" \n", items, provider.ChatDialect{Media: provider.ChatImages})
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, len(messages), 4)
	if strings.Contains(encoded(t, messages), "reasoning_content") {
		t.Fatal("OpenAI received reasoning_content")
	}
}

func TestRequestLimitsReasoningAndTools(t *testing.T) {
	limit, outputCap := uint32(8000), uint32(1024)
	for _, tc := range []struct {
		request provider.InferenceRequest
		want    *uint32
	}{{
		provider.InferenceRequest{},
		nil,
	}, {
		provider.InferenceRequest{
			OutputLimit: &limit,
		},
		&limit,
	}, {
		provider.InferenceRequest{
			OutputCap: &outputCap,
		},
		&outputCap,
	}, {
		provider.InferenceRequest{
			OutputLimit: &limit,
			OutputCap:   &outputCap,
		},
		&outputCap,
	}, {
		provider.InferenceRequest{
			OutputLimit: &outputCap,
			OutputCap:   &limit,
		},
		&outputCap,
	}} {
		requireEqual(t, tc.request.MaxOutputTokens(), tc.want)
	}
	requireEqual(t, *provider.AnthropicRequestLimits(core.Model{ContextWindow: 200000}).Images, uint32(100))
	requireEqual(t, *provider.AnthropicRequestLimits(core.Model{ContextWindow: 200001}).Images, uint32(600))
	off := core.ThinkingSummary("off")
	for _, tc := range []struct {
		thinking core.ThinkingConfig
		off      provider.SummaryOff
		want     string
	}{
		{&core.EffortConfig{Effort: "high", Summary: &off}, provider.SummaryOmitted, `{"effort":"high"}`},
		{&core.EffortConfig{Effort: "high", Summary: &off}, provider.SummaryAuto, `{"effort":"high","summary":"auto"}`},
		{&core.EffortConfig{Effort: "none"}, provider.SummaryAuto, `{"effort":"none"}`},
		{&core.AdaptiveConfig{Effort: "max"}, provider.SummaryOmitted, `{"effort":"max","summary":"auto"}`},
		{&core.BudgetConfig{BudgetTokens: 1000}, provider.SummaryOmitted, `null`},
		{&core.DisabledConfig{}, provider.SummaryOmitted, `null`},
	} {
		requireEqual(t, encoded(t, provider.ResponsesReasoning(tc.thinking, tc.off)), tc.want)
	}
	tool := provider.ToolDefinition{
		Name:        "run",
		Description: "execute",
		InputSchema: []byte(`{"type":"object","properties":{"z":{},"a":{}}}`),
	}
	requireEqual(
		t,
		encoded(t, provider.NewResponsesTool(tool, true)),
		`{"type":"function","name":"run","description":"execute",`+
			`"parameters":{"type":"object","properties":{"z":{},"a":{}}},`+
			`"strict":null}`,
	)
	if strings.Contains(encoded(t, provider.NewResponsesTool(tool, false)), "strict") {
		t.Fatal("unexpected strict")
	}
}

func TestSSECancellationAndEarlyExitCloseBody(t *testing.T) {
	for _, cancelRun := range []bool{false, true} {
		t.Run(map[bool]string{false: "consumer stops", true: "context cancels"}[cancelRun], func(t *testing.T) {
			vendor := providertest.StartVendor(t)
			response := providertest.EventStream("data: first\n\n")
			response.Ending = providertest.Open
			vendor.Respond(response)
			ctx, cancel := context.WithCancel(t.Context())
			defer cancel()
			request, err := http.NewRequestWithContext(ctx, http.MethodGet, vendor.URL("/stream"), nil)
			if err != nil {
				t.Fatal(err)
			}
			received, err := vendor.Client().Do(request)
			if err != nil {
				t.Fatal(err)
			}
			count := 0
			for data, err := range provider.SSEData(ctx, received.Body) {
				if err != nil {
					t.Fatal(err)
				}
				requireEqual(t, data, "first")
				count++
				if !cancelRun {
					break
				}
				cancel()
			}
			requireEqual(t, count, 1)
			vendor.Disconnected(t.Context())
		})
	}
}

func TestEndpointAndBodyBuild(t *testing.T) {
	for _, path := range []string{"/v1", "/v1/", "/v1/responses/"} {
		base, err := url.Parse("https://example.invalid" + path + "?key=secret#frag")
		if err != nil {
			t.Fatal(err)
		}
		requireEqual(
			t,
			provider.EndpointURL(base, "/responses").String(),
			"https://example.invalid/v1/responses?key=secret#frag",
		)
	}
	body, err := provider.EncodeBody(
		t.Context(),
		"Acme",
		func() ([]byte, error) { return provider.JSONBody(map[string]string{"text": "<>&\u2028\u2029"}) },
	)
	if err != nil {
		t.Fatal(err)
	}
	requireEqual(t, string(body), "{\"text\":\"<>&\u2028\u2029\"}")
	_, err = provider.EncodeBody(t.Context(), "Acme", func() ([]byte, error) { panic("credential must not appear") })
	var failure *provider.Failure
	if !errors.As(err, &failure) || failure.Code != nil || strings.Contains(err.Error(), "credential") {
		t.Fatalf("%v", err)
	}
	// An unreadable HTTP body is recorded as empty, preserving status and headers.
	response := &http.Response{
		StatusCode: 503,
		Header:     http.Header{"Retry-After": []string{"5"}},
		Body:       io.NopCloser(io.MultiReader(strings.NewReader("partial"), failedReader{errors.New("broken")})),
	}
	refused := provider.HTTPFailure(
		t.Context(),
		response,
		"Acme",
		provider.ReadHTTPFailure,
		providertest.FixedClock(now),
	)
	requireEqual(t, provider.ReadHTTPRecord(refused.Diagnostics).Body, "")
}

func TestToolArgumentsReadVendorJSONWithoutLosingObjectOrder(t *testing.T) {
	for _, tc := range []struct{ input, want string }{
		{`{"z":1,"a":{},"z":2}`, `{"z":2,"a":{}}`},
		{`[1.0,-0,1e0]`, `[1.0,-0.0,1.0]`},
		{`[0.000001,1e16]`, `[1e-6,1e+16]`},
		{`"broken {"`, `broken {`},
	} {
		got, err := provider.ToolArguments([]byte(tc.input))
		if err != nil {
			t.Fatal(err)
		}
		requireEqual(t, got, tc.want)
	}
	for _, input := range []string{`"\ud800"`, `1e999`, strings.Repeat("[", 128) + "0" + strings.Repeat("]", 128)} {
		if _, err := provider.ToolArguments([]byte(input)); err == nil {
			t.Fatalf("accepted invalid vendor JSON %q", input)
		}
	}
}
