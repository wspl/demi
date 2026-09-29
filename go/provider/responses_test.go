package provider_test

import (
	"context"
	"encoding/json/jsontext"
	"net/http"
	"reflect"
	"slices"
	"strings"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/provider/providertest"
)

func responseEvents(t *testing.T, frames ...jsontext.Value) []provider.ProviderEvent {
	t.Helper()
	server := providertest.NewMockVendor(t, providertest.MockResponse{Status: 200, Chunks: []string{providertest.SSEBody(frames...)}})
	request, err := http.NewRequestWithContext(t.Context(), http.MethodGet, server.Server.URL, nil)
	if err != nil {
		t.Fatal(err)
	}
	response, err := server.Server.Client().Do(request)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	return slices.Collect(provider.MapResponsesSSE(t.Context(), response.Body, provider.Vendor{Label: "Codex", Reader: provider.ReadHTTPFailure, Clock: providertest.FixedClock{}}, "codex:"))
}
func TestResponsesThinkingReplayToolsAndUsage(t *testing.T) {
	events := responseEvents(t,
		jsontext.Value(`{"type":"response.created"}`),
		jsontext.Value(`{"type":"response.output_item.added","item":{"type":"reasoning","id":"rs_1"}}`),
		jsontext.Value(`{"type":"response.reasoning_summary_text.delta","delta":"think"}`),
		jsontext.Value(`{"type":"response.output_item.done","item":{"type":"reasoning","id":"rs_1","encrypted_content":"enc","summary":[{"type":"summary_text","text":"thought"}],"status":"completed"}}`),
		jsontext.Value(`{"type":"response.output_text.delta","delta":"hello "}`),
		jsontext.Value(`{"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"shell_exec","arguments":""}}`),
		jsontext.Value(`{"type":"response.function_call_arguments.delta","delta":"{\"script\":"}`),
		jsontext.Value(`{"type":"response.function_call_arguments.done","arguments":"{\"script\":\"pwd\"}"}`),
		jsontext.Value(`{"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"shell_exec"}}`),
		jsontext.Value(`{"type":"response.completed","response":{"usage":{"input_tokens":100,"output_tokens":7,"input_tokens_details":{"cached_tokens":60,"cache_write_tokens":25}}}}`), jsontext.Value(`bad`))
	if len(events) != 6 {
		t.Fatalf("events: %#v", events)
	}
	signature, ok := events[2].(provider.ThinkingSignature)
	if !ok {
		t.Fatalf("signature: %#v", events[2])
	}
	for _, part := range []string{`codex:`, `"status":"completed"`, `"type":"summary_text"`, `"encrypted_content":"enc"`} {
		if !strings.Contains(signature.Signature, part) {
			t.Fatal(signature)
		}
	}
	want := []provider.ProviderEvent{provider.ThinkingStart{}, provider.ThinkingDelta{Text: "think"}, events[2], provider.TextDelta{Text: "hello "}, provider.ToolCall{ToolUseID: "call_1|fc_1", ToolName: "shell_exec", Input: jsontext.Value(`{"script":"pwd"}`)}, provider.Response{Usage: core.TokenUsage{InputTokens: 15, OutputTokens: 7, CacheReadTokens: 60, CacheWriteTokens: 25}}}
	if !reflect.DeepEqual(events, want) {
		t.Fatalf("events: %#v", events)
	}
}
func TestResponsesFinishedTextAndEOF(t *testing.T) {
	events := responseEvents(t,
		jsontext.Value(`{"type":"response.output_item.done","item":{"type":"reasoning","summary":[],"content":[{"text":"raw"}]}}`),
		jsontext.Value(`{"type":"response.output_text.delta","delta":"因为"}`),
		jsontext.Value(`{"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"because"}]}}`),
		jsontext.Value(`{"type":"response.output_item.done","item":{"type":"message","content":[{"type":"refusal","refusal":"no"},{"type":"unknown","bad":42}]}}`),
		jsontext.Value(`{"type":"response.output_item.done","item":{"type":"web_search_call","content":42}}`), jsontext.Value(`[DONE]`))
	var texts []string
	for _, event := range events {
		switch event := event.(type) {
		case provider.TextDelta:
			texts = append(texts, event.Text)
		case provider.ThinkingDelta:
			texts = append(texts, event.Text)
		}
	}
	if !reflect.DeepEqual(texts, []string{"raw", "因为", "no"}) {
		t.Fatal(texts, events)
	}
	if !reflect.DeepEqual(events[len(events)-1], provider.Response{}) {
		t.Fatal(events)
	}
}
func TestResponsesFailuresValidateMappedFieldsAndKeepIDs(t *testing.T) {
	for _, tc := range []struct {
		frame, field string
		code         provider.ErrorCode
	}{
		{`{"type":"response.failed","response":{"error":{"code":"context_length_exceeded","message":"too long"}}}`, "too long", provider.ContextLengthExceeded},
		{`{"type":"response.failed"}`, "Codex response failed", ""},
		{`{"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}`, "max_output_tokens", provider.ContextLengthExceeded},
		{`{"type":"response.incomplete","response":{"incomplete_details":{"reason":"content_filter"}}}`, "content_filter", provider.Incomplete},
		{`{"type":"error","code":"server_error","message":"backend failed"}`, "backend failed", provider.Overloaded},
		{`{"type":"error","message":"upstream failed","code":500}`, "upstream failed", ""},
		{`{"type":"error"}`, "Codex stream error", ""},
		{`{"type":"response.output_item.done","item":{"type":"message","content":42}}`, "content", ""},
		{`{"type":"response.output_text.delta","delta":42}`, "delta", ""},
		{`{"type":"response.completed","response":{"usage":{"input_tokens":12.5}}}`, "input_tokens", ""},
		{`{"type":"response.output_item.done","item":{"type":"reasoning","summary":[{"type":"summary_text"}]}}`, "text", ""},
		{`{"delta":"no type"}`, "type", ""},
	} {
		events := responseEvents(t, jsontext.Value(tc.frame), jsontext.Value(`bad`))
		if len(events) != 1 {
			t.Fatal(events)
		}
		failure, ok := events[0].(provider.FailureEvent)
		if !ok || failure.Failure.Code != tc.code || !strings.Contains(failure.Failure.Message, tc.field) || *failure.Failure.Diagnostics.Upstream != tc.frame {
			t.Fatalf("%s: %#v", tc.frame, events)
		}
	}
	events := responseEvents(t, jsontext.Value(`{"type":"response.failed","response":{"id":"resp-1","error":{"code":"server_error","message":"Failed. Include the request ID req-1 in your message."}}}`))
	diagnostics := events[0].(provider.FailureEvent).Failure.Diagnostics
	if *diagnostics.ProviderRequestID != "req-1" || *diagnostics.ProviderResponseID != "resp-1" {
		t.Fatal(diagnostics)
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if got := slices.Collect(provider.MapResponsesSSE(ctx, strings.NewReader("data: {}\n\n"), provider.Vendor{}, "")); len(got) != 0 {
		t.Fatal(got)
	}
}

// Cost: three scripted SSE streams; retained vendor members must survive both stream and request boundaries.
func TestReasoningRetainsUnknownMemberOrderAndRejectsDuplicates(t *testing.T) {
	events := responseEvents(t, jsontext.Value(`{"type":"response.output_item.done","item":{"type":"reasoning","id":null,"summary":[{"z_part":1,"text":"thought","a_part":2}],"z_item":{"inside":true},"a_item":[1,2]}}`))
	var signature string
	for _, event := range events {
		if value, ok := event.(provider.ThinkingSignature); ok {
			signature = value.Signature
		}
	}
	if signature == "" {
		t.Fatalf("missing signature: %#v", events)
	}
	items := provider.ResponsesInput([]provider.InferenceItem{provider.AssistantThinking{Signature: &signature}}, provider.ResponsesDialect{SignatureTag: "codex:", Reasoning: provider.ReasoningWhole})
	if len(items) != 1 {
		t.Fatalf("missing replay: %s", signature)
	}
	replay := string(items[0])
	for _, pair := range [][2]string{{`"z_item"`, `"a_item"`}, {`"z_part"`, `"a_part"`}} {
		left, right := strings.Index(replay, pair[0]), strings.Index(replay, pair[1])
		if left < 0 || right < left {
			t.Fatalf("retained member order changed: %s", replay)
		}
	}
	if strings.Contains(replay, `"id"`) {
		t.Fatalf("null optional member was replayed: %s", replay)
	}
	for _, item := range []string{
		`{"type":"reasoning","extra":1,"extra":2}`,
		`{"type":"reasoning","summary":[{"text":"thought","extra":1,"extra":2}]}`,
	} {
		events := responseEvents(t, jsontext.Value(`{"type":"response.output_item.done","item":`+item+`}`))
		if len(events) != 1 {
			t.Fatalf("duplicate members accepted: %#v", events)
		}
		if _, ok := events[0].(provider.FailureEvent); !ok {
			t.Fatalf("duplicate members accepted: %#v", events)
		}
	}
}
