package claudecode

import (
	"encoding/json"
	"strings"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
)

func TestMCPListsToolsAndSingleUnstreamedCall(t *testing.T) {
	r, p := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(v map[string]json.RawMessage) {
			if c.initialized(v) {
				checkJSON(t, v["request"], `{"subtype":"initialize","sdkMcpServers":["main"],"systemPrompt":"system"}`)
				return
			}
			if string(v["type"]) == `"user"` {
				c.handshake()
				c.mcp("list", map[string]any{"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
				c.mcp("ping", map[string]any{"jsonrpc": "2.0", "id": 2, "method": "ping"})
				c.call("call-1", 3, "toolu_1", "pwd")
				return
			}
			id, reply := mcpResponse(t, v)
			switch id {
			case "mcp-init", "mcp-initialized":
				// The scripted process validates the shared handshake replies.
			case "ping":
				checkJSON(t, reply["result"], `{}`)
			case "list":
				checkJSON(t, reply["result"], `{"tools":[{"name":"shell_exec","description":"Execute a shell script","inputSchema":{"type":"object","properties":{"script":{"type":"string"}},"required":["script"],"additionalProperties":false}}]}`)
			default:
				t.Fatalf("unexpected reply %s", id)
			}
		}
	})
	req := withTools(request(user("hi")))
	events := collect(t.Context(), r, req)
	equal(t, events, []provider.Event{&provider.ToolCall{ToolUseID: "toolu_1", ToolName: "shell_exec", Input: json.RawMessage(`{"script":"pwd"}`)}})
	c := p.starts[0]
	equal(t, len(c.input), 6)
	c.onWrite = func(v map[string]json.RawMessage) {
		id, reply := mcpResponse(t, v)
		equal(t, id, "call-1")
		equal(t, decoded(t, reply["jsonrpc"]), "2.0")
		equal(t, decoded(t, reply["id"]), float64(3))
		equal(t, len(reply), 3)
		checkJSON(t, reply["result"], `{"content":[{"type":"text","text":"/tmp"}],"isError":false}`)
		c.text("after the tool")
		c.result(2, 4)
	}
	req.Items = append(req.Items, toolUse("toolu_1", "pwd"), toolOutput("toolu_1", "/tmp"))
	equal(t, collect(t.Context(), r, req), []provider.Event{textEvent("after the tool"), response(2, 4)})
	equal(t, len(c.signals), 0)
}
func TestWholeBatchBeforeAnswersAndStoredLaterResult(t *testing.T) {
	r, p := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(v map[string]json.RawMessage) {
			if c.initialized(v) {
				return
			}
			if string(v["type"]) == `"user"` {
				c.handshake()
				c.say(startLine)
				c.text("running both")
				c.say(toolLine("toolu_alpha", "printf alpha"))
				c.call("call-alpha", 2, "toolu_alpha", "printf alpha")
				c.say(toolLine("toolu_beta", "printf beta"))
				c.say(stopLine)
			}
		}
	})
	req := withTools(request(user("run both")))
	events := collect(t.Context(), r, req)
	equal(t, events, []provider.Event{textEvent("running both"), &provider.ToolCall{ToolUseID: "toolu_alpha", ToolName: "shell_exec", Input: json.RawMessage(`{"script":"printf alpha"}`)}, &provider.ToolCall{ToolUseID: "toolu_beta", ToolName: "shell_exec", Input: json.RawMessage(`{"script":"printf beta"}`)}})
	c := p.starts[0]
	equal(t, len(c.input), 4)
	var replies []string
	c.onWrite = func(v map[string]json.RawMessage) {
		id, reply := mcpResponse(t, v)
		replies = append(replies, id)
		switch id {
		case "call-alpha":
			checkJSON(t, reply["result"], `{"content":[{"type":"text","text":"alpha done"}],"isError":false}`)
			c.call("call-beta", 2, "toolu_beta", "printf beta")
		case "call-beta":
			checkJSON(t, reply["result"], `{"content":[{"type":"text","text":"beta done"}],"isError":false}`)
			c.result(3, 5)
		default:
			t.Fatalf("unexpected reply %s", id)
		}
	}
	req.Items = append(req.Items, toolUse("toolu_alpha", "printf alpha"), toolUse("toolu_beta", "printf beta"), toolOutput("toolu_alpha", "alpha done"), toolOutput("toolu_beta", "beta done"))
	equal(t, collect(t.Context(), r, req), []provider.Event{response(3, 5)})
	equal(t, replies, []string{"call-alpha", "call-beta"})
	equal(t, len(p.starts), 1)
}
func TestMCPResultImagesAndErrors(t *testing.T) {
	r, p := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(v map[string]json.RawMessage) {
			if c.initialized(v) {
				return
			}
			if string(v["type"]) == `"user"` {
				c.handshake()
				c.mcp("call-1", map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": map[string]any{"name": "shell_exec", "arguments": map[string]any{"script": "shot"}}})
			}
		}
	})
	req := withTools(request(user("look")))
	events := collect(t.Context(), r, req)
	equal(t, len(events), 1)
	call, ok := events[0].(*provider.ToolCall)
	if !ok {
		t.Fatalf("%#v", events)
	}
	if !strings.HasPrefix(call.ToolUseID, "mcp-control-") {
		t.Fatal(call.ToolUseID)
	}
	checkJSON(t, call.Input, `{"script":"shot"}`)
	c := p.starts[0]
	c.onWrite = func(v map[string]json.RawMessage) {
		id, reply := mcpResponse(t, v)
		equal(t, id, "call-1")
		checkJSON(t, reply["result"], `{"content":[{"type":"text","text":"captured"},{"type":"image","data":"AQID","mimeType":"image/png"},{"type":"text","text":"[video:video/mp4]"}],"isError":true}`)
		c.result(1, 1)
	}
	req.Items = append(req.Items, &provider.ToolUse{ModelID: "claude-test", ToolUseID: call.ToolUseID, ToolName: call.ToolName, Input: call.Input}, &provider.ToolResult{ToolUseID: call.ToolUseID, IsError: true, Output: []provider.ResultPart{&provider.TextPart{Text: "captured"}, &provider.ResultImage{Bytes: provider.MediaBytes{Data: []byte{1, 2, 3}, MediaType: "image/png"}}, &provider.ResultVideo{Bytes: provider.MediaBytes{MediaType: "video/mp4"}}}})
	equal(t, collect(t.Context(), r, req), []provider.Event{response(1, 1)})
}
func TestMalformedCallAndUnknownControlRefused(t *testing.T) {
	r, p := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(v map[string]json.RawMessage) {
			if c.initialized(v) {
				return
			}
			if string(v["type"]) == `"user"` {
				c.handshake()
				c.mcp("call-1", map[string]any{"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": map[string]any{"arguments": map[string]any{}}})
				return
			}
			var outer struct {
				ID      string `json:"request_id"`
				Subtype string `json:"subtype"`
			}
			if err := json.Unmarshal(v["response"], &outer); err != nil {
				t.Fatal(err)
			}
			if outer.ID == "hook-1" {
				equal(t, outer.Subtype, "error")
				c.text("went on")
				c.result(1, 1)
				return
			}
			id, reply := mcpResponse(t, v)
			if id == "call-1" {
				var e struct {
					Code int `json:"code"`
				}
				if err := json.Unmarshal(reply["error"], &e); err != nil {
					t.Fatal(err)
				}
				equal(t, e.Code, -32601)
				c.say(`{"type":"control_request","request_id":"hook-1","request":{"subtype":"hook_callback","callback_id":"x"}}`)
			}
		}
	})
	equal(t, collect(t.Context(), r, withTools(request(user("hi")))), []provider.Event{textEvent("went on"), response(1, 1)})
	equal(t, len(p.starts[0].signals), 0)
}
func TestMissingAndUnaskedBatchResults(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r, p := fixture(t, func(c *scriptedCLI) {
			c.onWrite = func(v map[string]json.RawMessage) {
				if c.initialized(v) {
					return
				}
				c.say(startLine)
				c.say(toolLine("toolu_alpha", "printf alpha"))
				c.say(toolLine("toolu_beta", "printf beta"))
				c.say(stopLine)
			}
		})
		first := withTools(request(user("run both")))
		equal(t, len(collect(t.Context(), r, first)), 2)
		partial := withTools(request(user("run both"), toolOutput("toolu_alpha", "alpha done")))
		f := failure(t, collect(t.Context(), r, partial))
		equal(t, f.Message, "Claude Code provider missing tool_result for SDK MCP tool_use toolu_beta")
		equal(t, p.starts[0].signals, []host.Signal{host.Terminate})

		equal(t, len(collect(t.Context(), r, first)), 2)
		equal(t, len(p.starts), 2)
		c := p.starts[1]
		both := withTools(request(user("run both"), toolOutput("toolu_alpha", "alpha done"), toolOutput("toolu_beta", "beta done")))
		ended := make(chan []provider.Event, 1)
		go func() { ended <- collect(t.Context(), r, both) }()
		synctest.Wait()
		c.finish(host.ProcessEnd{Kind: host.ProcessExited})
		f = failure(t, <-ended)
		equal(t, f.Message, "Claude Code exited before requesting SDK MCP tool result for toolu_alpha, toolu_beta")
	})
}
func TestLeftoverOutputBelongsToNoRequest(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		r, p := fixture(t, func(c *scriptedCLI) {
			c.onWrite = func(v map[string]json.RawMessage) {
				if c.initialized(v) {
					return
				}
				if string(v["type"]) == `"user"` {
					c.handshake()
					c.say(startLine)
					c.say(toolLine("toolu_1", "pwd"))
					c.call("call-1", 2, "toolu_1", "pwd")
					c.say(stopLine)
				}
			}
		})
		req := withTools(request(user("run it")))
		collect(t.Context(), r, req)
		c := p.starts[0]
		var inputOrder []string
		c.onWrite = func(v map[string]json.RawMessage) {
			if string(v["type"]) == `"user"` {
				inputOrder = append(inputOrder, "steer")
				return
			}
			id, _ := mcpResponse(t, v)
			equal(t, id, "call-1")
			inputOrder = append(inputOrder, id)
			c.text("done")
			c.result(1, 1)
			c.text("listed it")
			c.result(2, 2)
		}
		req.Items = append(req.Items, toolUse("toolu_1", "pwd"), toolOutput("toolu_1", "/tmp"), &provider.UserSteer{Content: []provider.UserPart{&provider.TextPart{Text: "also list it"}}})
		equal(t, collect(t.Context(), r, req), []provider.Event{textEvent("done"), response(1, 1)})
		equal(t, inputOrder, []string{"steer", "call-1"})
		synctest.Wait()
		c.onWrite = func(v map[string]json.RawMessage) {
			checkJSON(t, v["message"], `{"role":"user","content":[{"type":"text","text":"next"}]}`)
			c.text("next answer")
			c.result(3, 3)
		}
		req.Items = append(req.Items, user("next"))
		equal(t, collect(t.Context(), r, req), []provider.Event{textEvent("next answer"), response(3, 3)})
	})
}

// These scenarios cover the SDK behavior formerly supplied by rmcp, in addition
// to the provider's Rust scenarios. All IO is the in-memory process script.
func TestMCPProtocolNegotiationAndDefaults(t *testing.T) {
	for _, version := range []string{"2024-11-05", "2025-06-18", "2099-01-01"} {
		t.Run(version, func(t *testing.T) {
			r, _ := fixture(t, func(c *scriptedCLI) {
				c.onWrite = func(v map[string]json.RawMessage) {
					if c.initialized(v) {
						return
					}
					if string(v["type"]) == `"user"` {
						c.mcp("init", map[string]any{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": map[string]any{"protocolVersion": version, "capabilities": map[string]any{}, "clientInfo": map[string]any{"name": "test", "version": "1"}}})
						return
					}
					id, reply := mcpResponse(t, v)
					if id == "init" {
						want := version
						if version == "2099-01-01" {
							want = "2025-11-25"
						}
						var result struct {
							Version string `json:"protocolVersion"`
						}
						if err := json.Unmarshal(reply["result"], &result); err != nil {
							t.Fatal(err)
						}
						equal(t, result.Version, want)
						for i, method := range []string{"resources/list", "resources/templates/list", "prompts/list"} {
							c.mcp(method, map[string]any{"jsonrpc": "2.0", "id": i + 2, "method": method})
						}
						c.mcp("complete", map[string]any{"jsonrpc": "2.0", "id": 5, "method": "completion/complete", "params": map[string]any{"ref": map[string]any{"type": "ref/prompt", "name": "x"}, "argument": map[string]any{"name": "x", "value": ""}}})
						return
					}
					switch id {
					case "resources/list":
						checkJSON(t, reply["result"], `{"resources":[]}`)
					case "resources/templates/list":
						checkJSON(t, reply["result"], `{"resourceTemplates":[]}`)
					case "prompts/list":
						checkJSON(t, reply["result"], `{"prompts":[]}`)
					case "complete":
						checkJSON(t, reply["result"], `{"completion":{"values":[]}}`)
						c.result(1, 1)
					default:
						t.Fatal(id)
					}
				}
			})
			equal(t, collect(t.Context(), r, withTools(request(user("hi")))), []provider.Event{response(1, 1)})
		})
	}
}

func TestMCPInlineMetadataAndFailedHandshake(t *testing.T) {
	for _, valid := range []bool{false, true} {
		t.Run(map[bool]string{false: "missing", true: "inline"}[valid], func(t *testing.T) {
			r, _ := fixture(t, func(c *scriptedCLI) {
				c.onWrite = func(v map[string]json.RawMessage) {
					if c.initialized(v) {
						return
					}
					if string(v["type"]) == `"user"` {
						params := map[string]any{}
						if valid {
							params["_meta"] = map[string]any{"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": map[string]any{}}
						}
						c.mcp("discover", map[string]any{"jsonrpc": "2.0", "id": 1, "method": "discover", "params": params})
						return
					}
					_, reply := mcpResponse(t, v)
					if valid {
						var result struct {
							Type     string   `json:"resultType"`
							Versions []string `json:"supportedVersions"`
							Scope    string   `json:"cacheScope"`
						}
						if err := json.Unmarshal(reply["result"], &result); err != nil {
							t.Fatal(err)
						}
						equal(t, result.Type, "complete")
						equal(t, result.Scope, "private")
						equal(t, result.Versions, []string{"2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25", "2026-07-28"})
					} else {
						var errReply struct {
							Code    int    `json:"code"`
							Message string `json:"message"`
						}
						if err := json.Unmarshal(reply["error"], &errReply); err != nil {
							t.Fatal(err)
						}
						equal(t, errReply.Code, -32602)
						if !strings.Contains(errReply.Message, "clientCapabilities") {
							t.Fatal(errReply.Message)
						}
					}
					c.result(1, 1)
				}
			})
			equal(t, collect(t.Context(), r, withTools(request(user("hi")))), []provider.Event{response(1, 1)})
		})
	}
}

func TestMCPPingWhileToolWaitsAndCancellation(t *testing.T) {
	r, _ := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(v map[string]json.RawMessage) {
			if c.initialized(v) {
				return
			}
			if string(v["type"]) == `"user"` {
				c.handshake()
				c.say(startLine)
				c.call("call", 9, "waiting", "pwd")
				c.mcp("ping", map[string]any{"jsonrpc": "2.0", "id": 10, "method": "ping"})
				return
			}
			id, reply := mcpResponse(t, v)
			switch id {
			case "mcp-init", "mcp-initialized":
			case "ping":
				checkJSON(t, reply["result"], `{}`)
				c.mcp("cancel", map[string]any{"jsonrpc": "2.0", "method": "notifications/cancelled", "params": map[string]any{"requestId": 9}})
			case "cancel":
				checkJSON(t, reply["result"], `{}`)
			case "call":
				checkJSON(t, reply["error"], `{"code":-32603,"message":"The tool call was cancelled"}`)
				c.result(1, 1)
			default:
				t.Fatal(id)
			}
		}
	})
	equal(t, collect(t.Context(), r, withTools(request(user("hi")))), []provider.Event{response(1, 1)})
}

func TestMCPMalformedEnvelopeIsControlError(t *testing.T) {
	for _, payload := range []string{`{"jsonrpc":"2.0"}`, `{"jsonrpc":"2.0","id":true,"method":"ping"}`, `{"jsonrpc":"2.0","id":1.5,"method":"ping"}`, `{"jsonrpc":"2.0","result":{}}`} {
		t.Run(payload, func(t *testing.T) {
			r, _ := fixture(t, func(c *scriptedCLI) {
				c.onWrite = func(v map[string]json.RawMessage) {
					if c.initialized(v) {
						return
					}
					if string(v["type"]) == `"user"` {
						c.handshake()
						c.mcp("bad", json.RawMessage(payload))
						return
					}
					var response struct {
						ID      string `json:"request_id"`
						Subtype string `json:"subtype"`
					}
					if err := json.Unmarshal(v["response"], &response); err != nil {
						t.Fatal(err)
					}
					if response.ID == "bad" {
						equal(t, response.Subtype, "error")
						c.result(1, 1)
					}
				}
			})
			equal(t, collect(t.Context(), r, withTools(request(user("hi")))), []provider.Event{response(1, 1)})
		})
	}
}
