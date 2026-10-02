package provider_test

import (
	"context"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

func TestChatSplitTextToolsAndUsage(t *testing.T) {
	got := mapped(t, true,
		`{"id":"chatcmpl-1","usage":null,"choices":[{"index":0,"delta":{"content":null},"finish_reason":null}]}`,
		`{"choices":[{"delta":{"content":"hi ","tool_calls":[{"index":0,"id":"call-1","function":{"name":"read_file","arguments":"{\"path\""}}]}}]}`,
		`{"choices":[{"delta":{"content":"there","tool_calls":[{"index":0,"function":{"arguments":":\"a.ts\"}"}}]}}]}`,
		`{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}`,
		`{"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":2}}}`,
		`[DONE]`, `{"choices":[{"delta":{"content":"never read"}}]}`)
	requireEqual(t, got, []provider.Event{&provider.TextDelta{Text: "hi "}, &provider.TextDelta{Text: "there"}, &provider.ToolCall{ToolUseID: "call-1", ToolName: "read_file", Input: []byte(`{"path":"a.ts"}`)}, &provider.Response{Usage: core.TokenUsage{InputTokens: 10, OutputTokens: 5, CacheReadTokens: 2}}})
}
func TestChatReasoningStartsOnce(t *testing.T) {
	got := mapped(t, true, `{"choices":[{"delta":{"role":"assistant","content":null,"reasoning_content":""}}]}`, `{"choices":[{"delta":{"content":null,"reasoning_content":"think "}}]}`, `{"choices":[{"delta":{"content":null,"reasoning_content":"more"}}]}`, `{"choices":[{"delta":{"content":"answer"}}]}`, `[DONE]`)
	requireEqual(t, got, []provider.Event{&provider.ThinkingStart{}, &provider.ThinkingDelta{Text: "think "}, &provider.ThinkingDelta{Text: "more"}, &provider.TextDelta{Text: "answer"}, &provider.Response{}})
}
func TestChatCallsAssembleAndFlush(t *testing.T) {
	got := mapped(t, true, `{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call-2","function":{"name":"bad","arguments":"{"}}]}}]}`, `{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"no_id"}}]}}]}`, `{"choices":[{"delta":{"tool_calls":[{"index":2,"id":"call-3","function":{"arguments":"{}"}}]}}]}`)
	requireEqual(t, got, []provider.Event{&provider.ToolCall{ToolUseID: "tool_call_0", ToolName: "no_id", Input: []byte(`{}`)}, &provider.ToolCall{ToolUseID: "call-2", ToolName: "bad", Input: []byte(`"{"`)}, &provider.Response{}})
	got = mapped(t, true, `{"choices":[{"delta":{"tool_calls":[{"id":"a","function":{"name":"first","arguments":"{}"}}]}}]}`, `{"choices":[{"delta":{"tool_calls":[{"id":"b","function":{"name":"second","arguments":"{}"}}]}}]}`, `[DONE]`)
	requireEqual(t, got[:2], []provider.Event{&provider.ToolCall{ToolUseID: "a", ToolName: "first", Input: []byte(`{}`)}, &provider.ToolCall{ToolUseID: "b", ToolName: "second", Input: []byte(`{}`)}})
}
func TestChatVendorFailureRecord(t *testing.T) {
	frame := `{"error":{"message":"quota exceeded","type":"insufficient_quota"}}`
	failure := failureOf(t, mapped(t, true, frame, `{"choices":[{"delta":{"content":"never read"}}]}`))
	requireEqual(t, failure.Message, "quota exceeded")
	requireEqual(t, *failure.Code, provider.RateLimit)
	requireEqual(t, *failure.Diagnostics.ProviderCode, "insufficient_quota")
	requireEqual(t, *failure.Diagnostics.Upstream, frame)
	requireEqual(t, failure.Diagnostics.Source, core.FailureSource("stream"))
	unnamed := failureOf(t, mapped(t, true, `{"error":{}}`))
	requireEqual(t, unnamed.Message, "Grok Build stream error")
	if unnamed.Code != nil {
		t.Fatal("unnamed code")
	}
}
func TestChatMalformedFields(t *testing.T) {
	for _, tc := range []struct{ frame, field string }{{`{"choices":[{"delta":{"content":{"text":"hi"}}}]}`, "content"}, {`{"choices":[{"delta":{"tool_calls":"none"}}]}`, "tool_calls"}, {`{"usage":{"prompt_tokens":"12"}}`, "prompt_tokens"}} {
		failure := failureOf(t, mapped(t, true, tc.frame))
		if failure.Code != nil || !strings.Contains(failure.Message, tc.field) {
			t.Fatalf("%+v", failure)
		}
		requireEqual(t, *failure.Diagnostics.Upstream, tc.frame)
	}
}
func TestChatCanceled(t *testing.T) {
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	requireEqual(t, mappedWith(ctx, t, true, provider.ReadHTTPFailure, `{"choices":[{"delta":{"content":"hi"}}]}`), []provider.Event{})
}
func TestResponsesThinkingTextToolsAndUsage(t *testing.T) {
	reasoning := `{"type":"reasoning","id":"rs_1","encrypted_content":"enc","summary":[{"type":"summary_text","text":"thought"}],"status":"completed"}`
	got := mapped(t, false,
		`{"type":"response.created","response":{"id":"resp-1"}}`,
		`{"type":"response.output_item.added","item":{"type":"reasoning","id":"rs_1"}}`,
		`{"type":"response.reasoning_summary_text.delta","delta":"think","sequence_number":3}`,
		`{"type":"response.output_item.done","item":`+reasoning+`}`,
		`{"type":"response.output_text.delta","delta":"hello "}`,
		`{"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"shell_exec","arguments":""}}`,
		`{"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"script\":"}`,
		`{"type":"response.function_call_arguments.done","item_id":"fc_1","arguments":"{\"script\":\"pwd\"}"}`,
		`{"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"shell_exec"}}`,
		`{"type":"response.completed","response":{"usage":{"input_tokens":100,"output_tokens":7,"input_tokens_details":{"cached_tokens":60,"cache_write_tokens":25}}}}`, `[DONE]`)
	if len(got) != 6 {
		t.Fatalf("%#v", got)
	}
	requireEqual(t, got[0], &provider.ThinkingStart{})
	requireEqual(t, got[1], &provider.ThinkingDelta{Text: "think"})
	signature, ok := got[2].(*provider.ThinkingSignature)
	if !ok || !strings.HasPrefix(signature.Signature, "codex:") {
		t.Fatalf("%#v", got[2])
	}
	requireEqual(t, jsonValue(t, strings.TrimPrefix(signature.Signature, "codex:")), jsonValue(t, reasoning))
	requireEqual(t, got[3], &provider.TextDelta{Text: "hello "})
	requireEqual(t, got[4], &provider.ToolCall{ToolUseID: "call_1|fc_1", ToolName: "shell_exec", Input: []byte(`{"script":"pwd"}`)})
	requireEqual(t, got[5], &provider.Response{Usage: core.TokenUsage{InputTokens: 15, OutputTokens: 7, CacheReadTokens: 60, CacheWriteTokens: 25}})
}
func TestResponsesFinishedTextOnlyWithoutDelta(t *testing.T) {
	got := mapped(t, false,
		`{"type":"response.output_item.added","item":{"type":"reasoning","id":"rs_1"}}`,
		`{"type":"response.output_item.done","item":{"type":"reasoning","id":"rs_1","content":[{"text":"raw reasoning"}],"encrypted_content":"enc"}}`,
		`{"type":"response.output_text.delta","delta":"因为"}`, `{"type":"response.output_text.delta","delta":"天空是蓝的"}`,
		`{"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"因为天空是蓝的"},{"type":"output_audio","data":"…"}]}}`,
		`{"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"and the next message"}]}}`,
		`{"type":"response.output_item.done","item":{"type":"web_search_call","status":"completed"}}`,
		`{"type":"response.output_item.done","item":{"type":"message","content":[{"type":"refusal","refusal":"no"}]}}`,
		`{"type":"response.completed"}`)
	texts := []string{}
	for _, event := range got {
		switch event := event.(type) {
		case *provider.TextDelta:
			texts = append(texts, event.Text)
		case *provider.ThinkingDelta:
			texts = append(texts, event.Text)
		case *provider.ThinkingStart, *provider.ThinkingSignature, *provider.RedactedThinking, *provider.ToolCall, *provider.Response, *provider.Error:
		}
	}
	requireEqual(t, texts, []string{"raw reasoning", "因为", "天空是蓝的", "and the next message", "no"})
	requireEqual(t, got[len(got)-1], &provider.Response{})
}
func TestResponsesEOFZeroUsage(t *testing.T) {
	requireEqual(t, mapped(t, false, `{"type":"response.output_text.delta","delta":"hi"}`), []provider.Event{&provider.TextDelta{Text: "hi"}, &provider.Response{}})
}
func TestResponsesFailureRecordAndReaderWait(t *testing.T) {
	frame := `{"type":"error","error":{"type":"usage_limit_reached","message":"The usage limit has been reached","resets_at":"soon"},"status_code":"429"}`
	reader := func(d *core.ProviderErrorDiagnostics, _ core.Timestamp) core.ProviderFailureFacts {
		requireEqual(t, *d.Upstream, frame)
		later := core.Timestamp("2026-09-18T14:01:00.000Z")
		return core.ProviderFailureFacts{RetryAt: &later}
	}
	failure := failureOf(t, mappedWith(t.Context(), t, false, reader, frame))
	requireEqual(t, failure.Message, "The usage limit has been reached")
	requireEqual(t, *failure.Code, provider.RateLimit)
	requireEqual(t, *failure.RetryAfter, time.Minute)
	requireEqual(t, *failure.Diagnostics.ProviderCode, "usage_limit_reached")
	requireEqual(t, *failure.Diagnostics.Upstream, frame)
	requireEqual(t, failure.Diagnostics.Source, core.FailureSource("stream"))
}
func TestResponsesFailureKinds(t *testing.T) {
	for _, tc := range []struct {
		frame, message string
		code           provider.ErrorCode
		raw            string
	}{
		{`{"type":"response.failed","response":{"error":{"code":"context_length_exceeded","message":"too long"}}}`, "too long", provider.ContextLengthExceeded, "context_length_exceeded"},
		{`{"type":"response.failed"}`, "Codex response failed", "", ""},
		{`{"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}`, "Incomplete Codex response returned, reason: max_output_tokens", provider.ContextLengthExceeded, ""},
		{`{"type":"response.incomplete","response":{"incomplete_details":{"reason":"content_filter"}}}`, "Incomplete Codex response returned, reason: content_filter", provider.Incomplete, ""},
		{`{"type":"error","code":"server_error","message":"backend failed"}`, "backend failed", provider.Overloaded, "server_error"},
		{`{"type":"error","error":{"type":"invalid_request_error","message":"Invalid prompt_cache_key"},"status":400}`, "Invalid prompt_cache_key", "invalid_request_error", "invalid_request_error"},
		{`{"type":"error","message":"upstream failed","code":500}`, "upstream failed", "", ""},
		{`{"type":"error"}`, "Codex stream error", "", ""},
	} {
		failure := failureOf(t, mapped(t, false, tc.frame, `{"type":"response.output_text.delta","delta":"never read"}`))
		requireEqual(t, failure.Message, tc.message)
		if tc.code == "" {
			if failure.Code != nil {
				t.Fatal("unexpected code")
			}
		} else {
			requireEqual(t, *failure.Code, tc.code)
		}
		if tc.raw == "" {
			if failure.Diagnostics.ProviderCode != nil {
				t.Fatal("unexpected vendor code")
			}
		} else {
			requireEqual(t, *failure.Diagnostics.ProviderCode, tc.raw)
		}
		requireEqual(t, *failure.Diagnostics.Upstream, tc.frame)
	}
}
func TestResponsesFailureIDs(t *testing.T) {
	failure := failureOf(t, mapped(t, false, `{"type":"response.failed","response":{"id":"resp-1","error":{"code":"server_error","message":"Failed. Please include the request ID req-1 in your message."}}}`))
	requireEqual(t, *failure.Code, provider.Overloaded)
	requireEqual(t, *failure.Diagnostics.ProviderRequestID, "req-1")
	requireEqual(t, *failure.Diagnostics.ProviderResponseID, "resp-1")
	failure = failureOf(t, mapped(t, false, `{"type":"error","error":{"message":"failed","request_id":"req-2"}}`))
	requireEqual(t, *failure.Diagnostics.ProviderRequestID, "req-2")
}
func TestResponsesMalformedMappedFields(t *testing.T) {
	for _, tc := range []struct{ frame, field string }{
		{`{"type":"response.output_item.done","item":{"type":"message","content":42}}`, "content"},
		{`{"type":"response.output_text.delta","delta":42}`, "delta"},
		{`{"type":"response.completed","response":{"usage":{"input_tokens":"100"}}}`, "input_tokens"},
		{`{"type":"response.completed","response":{"usage":{"input_tokens":12.5}}}`, "input_tokens"},
		{`{"type":"response.output_item.done","item":{"type":"reasoning","summary":[{"type":"summary_text"}]}}`, "text"},
	} {
		failure := failureOf(t, mapped(t, false, tc.frame))
		if failure.Code != nil || !strings.Contains(failure.Message, tc.field) {
			t.Fatalf("%+v", failure)
		}
		requireEqual(t, failure.Diagnostics.Source, core.FailureSource("stream"))
		requireEqual(t, *failure.Diagnostics.Upstream, tc.frame)
	}
	for _, text := range []string{`{"type":"response.queued","id":"r1"}`, " [DONE] "} {
		value, err := provider.DecodeResponsesFrame(text)
		if err != nil || value != nil {
			t.Fatalf("%v %v", value, err)
		}
	}
	if _, err := provider.DecodeResponsesFrame(`{"delta":"no type"}`); err == nil {
		t.Fatal("accepted missing type")
	}
}
func TestResponsesCanceled(t *testing.T) {
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	requireEqual(t, mappedWith(ctx, t, false, provider.ReadHTTPFailure, `{"type":"response.output_text.delta","delta":"hi"}`), []provider.Event{})
}
