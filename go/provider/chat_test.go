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

func chatEvents(t *testing.T, frames ...jsontext.Value) []provider.ProviderEvent {
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
	return slices.Collect(provider.MapChatSSE(t.Context(), response.Body, provider.Vendor{Label: "Vendor", Reader: provider.ReadHTTPFailure, Clock: providertest.FixedClock{}}))
}
func TestChatStreamMapsReasoningToolsAndUsage(t *testing.T) {
	events := chatEvents(t,
		jsontext.Value(`{"usage":null,"choices":[{"delta":{"content":null,"reasoning_content":""},"finish_reason":null}]}`),
		jsontext.Value(`{"choices":[{"delta":{"reasoning_content":"think ","content":"hi ","tool_calls":[{"index":0,"id":"call-1","function":{"name":"read","arguments":"{\"path\""}}]}}]}`),
		jsontext.Value(`{"choices":[{"delta":{"reasoning_content":"more","content":"there","tool_calls":[{"index":0,"function":{"arguments":":\"a.ts\"}"}}]}}]}`),
		jsontext.Value(`{"choices":[{"finish_reason":"tool_calls"}]}`),
		jsontext.Value(`{"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":2}}}`),
		jsontext.Value(`[DONE]`), jsontext.Value(`bad`))
	want := []provider.ProviderEvent{provider.ThinkingStart{}, provider.ThinkingDelta{Text: "think "}, provider.TextDelta{Text: "hi "}, provider.ThinkingDelta{Text: "more"}, provider.TextDelta{Text: "there"}, provider.ToolCall{ToolUseID: "call-1", ToolName: "read", Input: jsontext.Value(`{"path":"a.ts"}`)}, provider.Response{Usage: core.TokenUsage{InputTokens: 10, OutputTokens: 5, CacheReadTokens: 2}}}
	if !reflect.DeepEqual(events, want) {
		t.Fatalf("events: %#v", events)
	}
}
func TestChatEOFSortsCallsAndPreservesInvalidInput(t *testing.T) {
	events := chatEvents(t, jsontext.Value(`{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"second","function":{"name":"bad","arguments":"{"}},{"index":0,"function":{"name":"no_id"}},{"index":2,"id":"unnamed"}]}}]}`))
	want := []provider.ProviderEvent{provider.ToolCall{ToolUseID: "tool_call_0", ToolName: "no_id", Input: jsontext.Value(`{}`)}, provider.ToolCall{ToolUseID: "second", ToolName: "bad", Input: jsontext.Value(`"{"`)}, provider.Response{}}
	if !reflect.DeepEqual(events, want) {
		t.Fatalf("events: %#v", events)
	}
	sequential := chatEvents(t, jsontext.Value(`{"choices":[{"delta":{"tool_calls":[{"id":"a","function":{"name":"first"}},{"id":"b","function":{"name":"second"}}]}}]}`))
	if len(sequential) != 3 || sequential[0].(provider.ToolCall).ToolUseID != "a" || sequential[1].(provider.ToolCall).ToolUseID != "b" {
		t.Fatal(sequential)
	}
}
func TestChatFailuresAndCancellation(t *testing.T) {
	for _, tc := range []struct {
		frame, field string
		code         provider.ErrorCode
	}{
		{`{"error":{"message":"quota exceeded","type":"insufficient_quota"}}`, "quota", provider.RateLimit},
		{`{"error":{}}`, "Vendor stream error", ""},
		{`{"choices":[{"delta":{"content":{"text":"hi"}}}]}`, "content", ""},
		{`{"choices":[{"delta":{"tool_calls":"none"}}]}`, "tool_calls", ""},
		{`{"usage":{"prompt_tokens":"12"}}`, "prompt_tokens", ""},
		{`{"usage":{"prompt_tokens":1.5}}`, "prompt_tokens", ""},
		{`{"choices":[null]}`, "choices", ""},
	} {
		events := chatEvents(t, jsontext.Value(tc.frame), jsontext.Value(`bad`))
		if len(events) != 1 {
			t.Fatal(events)
		}
		failure, ok := events[0].(provider.FailureEvent)
		if !ok || failure.Failure.Code != tc.code || !strings.Contains(failure.Failure.Message, tc.field) || *failure.Failure.Diagnostics.Upstream != tc.frame {
			t.Fatalf("%s: %#v", tc.frame, events)
		}
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	events := slices.Collect(provider.MapChatSSE(ctx, strings.NewReader("data: {}\n\n"), provider.Vendor{}))
	if len(events) != 0 {
		t.Fatal(events)
	}
}
