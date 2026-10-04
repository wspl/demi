package anthropicapi_test

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/types"
)

func TestStreamMapping(t *testing.T) {
	events := eventsOf(t, recorded(
		`{"type":"message_start","message":{"usage":{"input_tokens":12,`+
			`"cache_read_input_tokens":2,"cache_creation_input_tokens":1,`+
			`"output_tokens":1}}}`,
		`{"type":"content_block_start","index":0,`+
			`"content_block":{"type":"thinking","thinking":"","signature":""}}`,
		`{"type":"ping"}`,
		`{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"plan"}}`,
		`{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig"}}`,
		`{"type":"content_block_stop","index":0}`,
		`{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}`,
		`{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"hello"}}`,
		`{"type":"content_block_stop","index":1}`,
		`{"type":"content_block_start","index":2,`+
			`"content_block":{"type":"tool_use","id":"toolu_1",`+
			`"name":"read_file","input":{}}}`,
		`{"type":"content_block_delta","index":2,`+
			`"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}`,
		`{"type":"content_block_delta","index":2,`+
			`"delta":{"type":"input_json_delta","partial_json":"\"a.ts\"}"}}`,
		`{"type":"content_block_stop","index":2}`,
		`{"type":"message_delta","delta":{"stop_reason":"tool_use"},`+
			`"usage":{"input_tokens":0,"output_tokens":5}}`,
		`{"type":"message_stop"}`,
	))
	equalEvents(
		t,
		events,
		[]provider.Event{
			&provider.ThinkingStart{},
			&provider.ThinkingDelta{Text: "plan"},
			&provider.ThinkingSignature{Signature: "anthropic:sig"},
			&provider.TextDelta{Text: "hello"},
			&provider.ToolCall{ToolUseID: "toolu_1", ToolName: "read_file", Input: json.RawMessage(`{"path":"a.ts"}`)},
			&provider.Response{
				Usage: types.TokenUsage{InputTokens: 12, OutputTokens: 5, CacheReadTokens: 2, CacheWriteTokens: 1},
			},
		},
	)
}

func TestToolInputPrecedence(t *testing.T) {
	events := eventsOf(t, recorded(
		`{"type":"content_block_start","index":0,`+
			`"content_block":{"type":"tool_use","id":"t0","name":"a","input":{"q":1}}}`,
		`{"type":"content_block_stop","index":0}`,
		`{"type":"content_block_start","index":1,`+
			`"content_block":{"type":"tool_use","id":"t1","name":"b"}}`,
		`{"type":"content_block_stop","index":1}`,
		`{"type":"content_block_start","index":2,`+
			`"content_block":{"type":"tool_use","id":"t2","name":"c","input":{}}}`,
		`{"type":"content_block_delta","index":2,`+
			`"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}`,
		`{"type":"content_block_stop","index":2}`,
		`{"type":"message_stop"}`,
	))
	equalEvents(t, events, []provider.Event{
		&provider.ToolCall{ToolUseID: "t0", ToolName: "a", Input: json.RawMessage(`{"q":1}`)},
		&provider.ToolCall{ToolUseID: "t1", ToolName: "b", Input: json.RawMessage(`{}`)},
		&provider.ToolCall{ToolUseID: "t2", ToolName: "c", Input: json.RawMessage(`"{\"path\":"`)},
		&provider.Response{},
	})
}

func TestRedactedThinking(t *testing.T) {
	equalEvents(
		t,
		eventsOf(
			t,
			recorded(
				`{"type":"content_block_start","index":0,`+
					`"content_block":{"type":"redacted_thinking","data":"EmwKAhgB"}}`,
				`{"type":"content_block_stop","index":0}`,
				`{"type":"message_stop"}`,
			),
		),
		[]provider.Event{&provider.RedactedThinking{Data: "anthropic:EmwKAhgB"}, &provider.Response{}},
	)
}

func TestEOFRespondsWithUsage(t *testing.T) {
	equalEvents(
		t,
		eventsOf(
			t,
			recorded(
				`{"type":"message_start","message":{"usage":{"input_tokens":7}}}`,
				`{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"partial"}}`,
			),
		),
		[]provider.Event{
			&provider.TextDelta{Text: "partial"},
			&provider.Response{Usage: types.TokenUsage{InputTokens: 7}},
		},
	)
}

func TestUnknownEventsAndBlocks(t *testing.T) {
	response := recorded(
		`{"type":"ping"}`,
		`{"type":"message_flavour","flavour":"new"}`,
		`{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"s1"}}`,
		`{"type":"content_block_delta","index":0,"delta":{"type":"citations_delta"}}`,
		`{"type":"content_block_stop","index":0}`,
		`{"type":"message_stop"}`,
	)
	response.Chunks = append(
		[][]byte{[]byte(": keep-alive\n\nevent: ping\ndata: {\"type\":\"ping\"}\n\ndata:\n\n")},
		response.Chunks...)
	equalEvents(t, eventsOf(t, response), []provider.Event{&provider.Response{}})
}

func TestMalformedKnownEvent(t *testing.T) {
	cases := []struct{ frame, field string }{
		{`{"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}`, "index"},
		{`{"type":"content_block_start","index":0,"content_block":{"type":"text","text":42}}`, "text"},
		{`{"type":"content_block_start","index":0,` +
			`"content_block":{"type":"tool_use","id":"","name":"a"}}`, "id"},
		{`{"type":"message_delta","usage":{"output_tokens":1.5}}`, "output_tokens"},
	}
	for _, c := range cases {
		t.Run(c.field, func(t *testing.T) {
			failure := onlyFailure(t, eventsOf(t, recorded(c.frame, `{"type":"message_stop"}`)))
			if failure.Code != nil || !strings.Contains(failure.Message, c.field) ||
				failure.Diagnostics.Source != "stream" ||
				*failure.Diagnostics.Upstream != c.frame {
				t.Fatalf("%+v", failure)
			}
		})
	}
}

func TestErrorEvent(t *testing.T) {
	frame := `  {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}  `
	events := eventsOf(
		t,
		recorded(
			`{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"partial"}}`,
			frame,
			`{"type":"message_stop"}`,
		),
	)
	if len(events) != 2 {
		t.Fatal(events)
	}
	equalEvents(t, events[:1], []provider.Event{&provider.TextDelta{Text: "partial"}})
	failure := onlyFailure(t, events[1:])
	if failure.Message != "Overloaded" || failure.Code == nil || *failure.Code != provider.Overloaded ||
		failure.RetryAfter != nil ||
		failure.Diagnostics.Source != "stream" ||
		*failure.Diagnostics.ProviderCode != "overloaded_error" ||
		*failure.Diagnostics.Upstream != frame {
		t.Fatalf("%+v", failure)
	}
	for _, c := range []struct {
		frame, message string
		code           provider.ErrorCode
	}{
		{
			`{"type":"error","error":{"type":"rate_limit_error",` +
				`"message":"Number of requests exceeded"}}`,
			"Number of requests exceeded",
			provider.RateLimit,
		},
		{
			`{"type":"error","error":{"type":"invalid_request_error","message":42}}`,
			"Anthropic API stream error",
			"invalid_request_error",
		},
		{`{"type":"error","message":"stream closed by the service"}`, "stream closed by the service", "error"},
	} {
		failure := onlyFailure(t, eventsOf(t, recorded(c.frame)))
		if failure.Message != c.message || failure.Code == nil || *failure.Code != c.code {
			t.Fatalf("%+v", failure)
		}
	}
}

func TestEveryStopReasonResponds(t *testing.T) {
	for _, reason := range []string{"end_turn", "tool_use", "max_tokens", "stop_sequence", "refusal", "pause_turn"} {
		t.Run(reason, func(t *testing.T) {
			equalEvents(
				t,
				eventsOf(
					t,
					recorded(
						`{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done"}}`,
						`{"type":"message_delta","delta":{"stop_reason":"`+
							reason+
							`"},"usage":{"output_tokens":2}}`,
						`{"type":"message_stop"}`,
					),
				),
				[]provider.Event{
					&provider.TextDelta{Text: "done"},
					&provider.Response{Usage: types.TokenUsage{OutputTokens: 2}},
				},
			)
		})
	}
}

// onlyFailure extracts the terminal failure of a failed run.
func onlyFailure(t *testing.T, events []provider.Event) provider.Failure {
	t.Helper()
	if len(events) != 1 {
		t.Fatalf("expected one failure: %#v", events)
	}
	event, ok := events[0].(*provider.Error)
	if !ok {
		t.Fatalf("expected failure: %#v", events[0])
	}
	return event.Failure
}

func TestBrokenStream(t *testing.T) {
	response := recorded(`{"type":"content_block_delta","index":0,` +
		`"delta":{"type":"text_delta","text":"partial"}}`)
	response.Ending = providertest.Broken
	events := eventsOf(t, response)
	if len(events) != 2 {
		t.Fatal(events)
	}
	equalEvents(t, events[:1], []provider.Event{&provider.TextDelta{Text: "partial"}})
	failure := onlyFailure(t, events[1:])
	if failure.Code == nil || *failure.Code != provider.Overloaded || failure.Diagnostics.Source != "transport" {
		t.Fatalf("%+v", failure)
	}
}
