package claudecode

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/version"
)

// These complete lines follow Rust input.rs and rmcp 3.4.1's Serialize
// declarations, including omitted optional fields. Expected bytes are literals,
// not produced by the Go encoders being tested. This scenario uses no real CLI.
func TestCLIWireBytes(t *testing.T) {
	var initializeID string
	r, p := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(v map[string]json.RawMessage) {
			if c.initialized(v) {
				initializeID = string(v["request_id"])
				return
			}
			if string(v["type"]) == `"user"` {
				c.handshake()
				c.mcp("list", json.RawMessage(`{"jsonrpc":"2.0","id":1,"method":"tools/list"}`))
				c.mcp("bad", json.RawMessage(`{"jsonrpc":"2.0","id":2,"method":"missing"}`))
				return
			}
			id, _ := mcpResponse(t, v)
			if id == "bad" {
				c.result(1, 1)
			}
		}
	})
	req := request(&provider.UserMessage{Content: []provider.UserPart{
		&provider.TextPart{Text: "<&>\u2028\u2029"},
		&provider.ImagePart{Medium: &provider.MediaBytes{MediaType: "image/png", Data: []byte("png")}},
		&provider.DocumentPart{
			Bytes:    provider.MediaBytes{MediaType: "application/pdf", Data: []byte("pdf")},
			FileName: "notes.pdf",
		},
	}})
	req.Tools = []provider.ToolDefinition{
		{
			Name:        "inspect",
			Description: "Read <&>",
			InputSchema: json.RawMessage(
				`{ "type": "object", "properties": { "z": { "type": "string" }, ` +
					`"a": { "type": "number" } }, "required": [ "z" ] }`,
			),
		},
	}
	equal(t, collect(t.Context(), r, req), []provider.Event{response(1, 1)})
	want := []string{
		`{"type":"control_request","request_id":` +
			initializeID +
			`,"request":{"subtype":"initialize","sdkMcpServers":["main"],` +
			`"systemPrompt":"system"}}`,
		`{"type":"user","message":{"role":"user",` +
			`"content":[{"type":"text","text":"<&>` + "\u2028\u2029" + `"},{"type":"image",` +
			`"source":{"type":"base64","media_type":"image/png","data":"cG5n"}` +
			`},{"type":"document","source":{"type":"base64",` +
			`"media_type":"application/pdf","data":"cGRm"},"title":"notes.pdf"}]}}`,
		`{"type":"control_response","response":{"subtype":"success",` +
			`"request_id":"mcp-init",` +
			`"response":{"mcp_response":{"jsonrpc":"2.0","id":0,` +
			`"result":{"protocolVersion":"2025-06-18",` +
			`"capabilities":{"tools":{}},"serverInfo":{"name":"demi","version":"` +
			version.Release +
			`"}}}}}}`,
		`{"type":"control_response","response":{"subtype":"success",` +
			`"request_id":"mcp-initialized",` +
			`"response":{"mcp_response":{"jsonrpc":"2.0","id":0,"result":{}}}}}`,
		`{"type":"control_response","response":{"subtype":"success",` +
			`"request_id":"list","response":{"mcp_response":{"jsonrpc":"2.0",` +
			`"id":1,"result":{"tools":[{"name":"inspect","description":"Read ` +
			`<&>","inputSchema":{"type":"object",` +
			`"properties":{"z":{"type":"string"},"a":{"type":"number"}},` +
			`"required":["z"]}}]}}}}}`,
		`{"type":"control_response","response":{"subtype":"success",` +
			`"request_id":"bad","response":{"mcp_response":{"jsonrpc":"2.0",` +
			`"id":2,"error":{"code":-32601,"message":"missing"}}}}}`,
	}
	equal(t, len(p.starts[0].input), len(want))
	for i, line := range want {
		equal(t, string(p.starts[0].input[i]), line+"\n")
	}
}

func TestToolValuesKeepReadOrderAndSerdeSpelling(t *testing.T) {
	const input = `{ "z": [ { "b": "\u003c&>\u2028", "a": 1e2 } ], "a": -0, "z": [ ` +
		`{ "b": "\u003c&>\u2028", "a": 1e2 } ] }`
	const want = `{"z":[{"b":"<&>` + "\u2028" + `","a":100.0}],"a":-0.0}`
	t.Run("MCP arguments", func(t *testing.T) {
		r, _ := fixture(t, func(c *scriptedCLI) {
			c.onWrite = func(v map[string]json.RawMessage) {
				if c.initialized(v) {
					return
				}
				if string(v["type"]) == `"user"` {
					c.handshake()
					c.mcp(
						"call",
						json.RawMessage(
							`{"jsonrpc":"2.0","id":1,"method":"tools/call",`+
								`"params":{"name":"shell_exec","arguments":`+
								input+
								`,"_meta":{"claudecode/toolUseId":"toolu_1"}}}`,
						),
					)
				}
			}
		})
		equal(
			t,
			collect(t.Context(), r, withTools(request(user("hi")))),
			[]provider.Event{
				&provider.ToolCall{ToolUseID: "toolu_1", ToolName: "shell_exec", Input: json.RawMessage(want)},
			},
		)
	})
	t.Run("assistant tool input", func(t *testing.T) {
		r, _ := fixture(t, func(c *scriptedCLI) {
			c.onWrite = func(v map[string]json.RawMessage) {
				if c.initialized(v) {
					return
				}
				if string(v["type"]) == `"user"` {
					c.say(
						`{"type":"assistant","message":{"content":[{"type":"tool_use",` +
							`"id":"toolu_1","name":"shell_exec","input":` +
							input +
							`}]}}`,
					)
					c.say(stopLine)
				}
			}
		})
		equal(
			t,
			collect(t.Context(), r, withTools(request(user("hi")))),
			[]provider.Event{
				&provider.ToolCall{ToolUseID: "toolu_1", ToolName: "shell_exec", Input: json.RawMessage(want)},
			},
		)
	})
	t.Run("transcript", func(t *testing.T) {
		r, p := fixture(t, func(c *scriptedCLI) { c.onWrite = func(map[string]json.RawMessage) { c.result(1, 1) } })
		equal(
			t,
			collect(
				t.Context(),
				r,
				request(&provider.ToolUse{ToolUseID: "toolu_1", ToolName: "shell_exec", Input: json.RawMessage(input)}),
			),
			[]provider.Event{response(1, 1)},
		)
		equal(
			t,
			string(p.starts[0].input[0]),
			`{"type":"user","message":{"role":"user",`+
				`"content":[{"type":"text","text":"Assistant: [Earlier in this `+
				`conversation I called the tool shell_exec with input: `+
				`{\"z\":[{\"b\":\"<&>`+"\u2028"+`\",\"a\":100.0}],\"a\":-0.0}."}]}}`+
				"\n",
		)
	})
}

func TestFramingFailureUsesLinesCodecReasonWithoutRecord(t *testing.T) {
	// The oversized case allocates one 64 MiB input to exercise the real bound.
	for _, tc := range []struct{ name, line, reason string }{
		{"UTF8", "\xff", "Unable to decode input as UTF8"},
		{"length", strings.Repeat("x", maxLineBytes+1), "max line length exceeded"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			events, c := answer(t, tc.line)
			f := failure(t, events)
			equal(t, f.Message, "Claude Code's output cannot be read: "+tc.reason)
			if f.Diagnostics != nil && f.Diagnostics.Upstream != nil {
				t.Fatal("framing failure retained a partial line")
			}
			equal(t, c.signals, []host.Signal{host.Terminate})
		})
	}
}
