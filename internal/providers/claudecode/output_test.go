package claudecode_test

import (
	"fmt"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
)

func TestWholeMessageTextAndReasoning(t *testing.T) {
	events, _ := answer(
		t,
		`{"type":"assistant","message":{"content":[{"type":"thinking",`+
			`"thinking":"considering","signature":"sig-1"},`+
			`{"type":"redacted_thinking","data":"opaque"},`+
			`{"type":"server_tool_use","id":"srv"},{"type":"text","text":""},`+
			`{"type":"text","text":"hello"}]}}`,
		doneLine,
	)
	equal(
		t,
		events,
		[]provider.Event{
			&provider.ThinkingStart{},
			&provider.ThinkingDelta{Text: "considering"},
			&provider.ThinkingSignature{Signature: "sig-1"},
			&provider.RedactedThinking{Data: "opaque"},
			textEvent("hello"),
			response(1, 1),
		},
	)
}

func TestStreamedPiecesDoNotRepeat(t *testing.T) {
	events, _ := answer(
		t,
		startLine,
		`{"type":"stream_event","event":{"type":"content_block_start",`+
			`"content_block":{"type":"thinking"}}}`,
		`{"type":"stream_event","event":{"type":"content_block_delta",`+
			`"delta":{"type":"thinking_delta","thinking":"hmm"}}}`,
		`{"type":"stream_event","event":{"type":"content_block_delta",`+
			`"delta":{"type":"thinking_delta"}}}`,
		`{"type":"stream_event","event":{"type":"content_block_delta",`+
			`"delta":{"type":"signature_delta","signature":"sig"}}}`,
		`{"type":"stream_event","event":{"type":"content_block_start",`+
			`"content_block":{"type":"text","text":""}}}`,
		`{"type":"stream_event","event":{"type":"content_block_delta",`+
			`"delta":{"type":"text_delta","text":"hi"}}}`,
		`{"type":"stream_event","event":{"type":"content_block_delta",`+
			`"delta":{"type":"input_json_delta","partial_json":"{"}}}`,
		`{"type":"stream_event","event":{"type":"message_delta","usage":{"output_tokens":3}}}`,
		`{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}`,
		stopLine,
		`{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}`,
		doneLine,
	)
	equal(
		t,
		events,
		[]provider.Event{
			&provider.ThinkingStart{},
			&provider.ThinkingDelta{Text: "hmm"},
			&provider.ThinkingSignature{Signature: "sig"},
			textEvent("hi"),
			response(1, 1),
		},
	)
}

func TestUnreadableLineRecordsAndCloses(t *testing.T) {
	for _, tc := range []struct {
		name    string
		lines   []string
		message string
	}{
		{
			"delta",
			[]string{
				`{"type":"stream_event","event":{"type":"content_block_delta",` +
					`"delta":{"type":"text_delta","text":{"value":"hi"}}}}`,
			},
			"text",
		},
		{"type", []string{`{"result":"ok"}`}, "type"},
		{
			"id",
			[]string{
				`{"type":"assistant","message":{"content":[{"type":"tool_use",` +
					`"name":"mcp__main__shell_exec","input":{}}]}}`,
			},
			"Invalid tool_use block from Claude Code",
		},
		{
			"unoffered",
			[]string{
				startLine,
				toolLine("toolu_1", "pwd"),
				stopLine,
			},
			"Claude Code called a tool, but the request offered none",
		},
		{"invalid_json", []string{`{"type":`}, "cannot read"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			events, c := answer(t, tc.lines...)
			f := failure(t, events)
			if tc.name == "id" || tc.name == "unoffered" {
				equal(t, f.Message, tc.message)
			} else if !strings.Contains(f.Message, tc.message) {
				t.Fatal(f.Message)
			}
			equal(t, f.Code, (*provider.ErrorCode)(nil))
			equal(t, *f.Diagnostics.Upstream, tc.lines[len(tc.lines)-1])
			equal(t, c.signals, []host.Signal{host.Terminate})
		})
	}
}

func TestCLIFailuresUseVendorWords(t *testing.T) {
	for _, tc := range []struct {
		line, message string
		code          provider.ErrorCode
		closed        bool
	}{
		{`{"type":"error","message":"rate limited, try later"}`, "rate limited, try later", provider.RateLimit, true},
		{`{"type":"error","message":"boom","code":"auth_expired"}`, "boom", provider.AuthExpired, true},
		{`{"type":"error","message":{"detail":"boom"}}`, "Claude Code error", "", true},
		{
			`{"type":"result","is_error":true,"result":"authentication ` +
				`failed","errors":[3]}`,
			"authentication failed",
			provider.AuthExpired,
			false,
		},
		{`{"type":"result","is_error":true,"result":{"text":1}}`, "Claude Code returned an error", "", false},
	} {
		t.Run(tc.message, func(t *testing.T) {
			events, c := answer(t, tc.line)
			f := failure(t, events)
			equal(t, f.Message, tc.message)
			if tc.code == "" {
				equal(t, f.Code, (*provider.ErrorCode)(nil))
			} else {
				equal(t, *f.Code, tc.code)
			}
			equal(t, c.closed, tc.closed)
			if tc.closed {
				equal(t, c.signals, []host.Signal{host.Terminate})
			} else {
				equal(t, len(c.signals), 0)
			}
		})
	}
}

func TestVendorRefusalUsesHTTPStatusOnce(t *testing.T) {
	for _, tc := range []struct {
		status int
		words  string
		code   provider.ErrorCode
	}{
		{
			400,
			"The request is refused.",
			"",
		}, {
			400,
			"prompt is too long: 250000 tokens",
			provider.ContextLengthExceeded,
		}, {
			401,
			"OAuth token has expired",
			provider.AuthExpired,
		}, {
			429,
			"Too many requests",
			provider.RateLimit,
		}, {
			529,
			"Overloaded",
			provider.Overloaded,
		},
	} {
		t.Run(fmt.Sprint(tc.status, tc.code), func(t *testing.T) {
			message := fmt.Sprintf("API Error: %d %s", tc.status, tc.words)
			notice, _ := provider.JSONBody(
				map[string]any{
					"type":  "assistant",
					"error": "unknown",
					"message": map[string]any{
						"model":   "<synthetic>",
						"content": []any{map[string]any{"type": "text", "text": message}},
					},
				},
			)
			result, _ := provider.JSONBody(
				map[string]any{"type": "result", "is_error": true, "result": message, "api_error_status": tc.status},
			)
			events, c := answer(t, string(notice), string(result))
			f := failure(t, events)
			equal(t, f.Message, message)
			equal(t, int(*f.Diagnostics.HTTPStatus), tc.status)
			if tc.code == "" {
				equal(t, f.Code, (*provider.ErrorCode)(nil))
			} else {
				equal(t, *f.Code, tc.code)
			}
			equal(t, len(c.signals), 0)
		})
	}
}
