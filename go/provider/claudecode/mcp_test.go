package claudecode

import (
	"context"
	"encoding/json/jsontext"
	"strings"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
)

// Cost: an in-memory SDK session; no CLI process or network connection.
func TestMCPPendingCallsRepliesAndEarlyResults(t *testing.T) {
	ctx, cancel := context.WithTimeout(t.Context(), 2*time.Minute)
	defer cancel()
	bridge, err := startMCP(ctx, []provider.ToolDefinition{{Name: "read", Description: "read a file", InputSchema: jsontext.Value(`{"type":"object"}`)}})
	if err != nil {
		t.Fatal(err)
	}
	defer bridge.close()
	pass := func(id, raw string) {
		t.Helper()
		if _, err := bridge.pass(ctx, id, []byte(raw)); err != nil {
			t.Fatal(err)
		}
	}
	reply := func(id string) string {
		t.Helper()
		event, err := bridge.next(ctx)
		if err != nil {
			t.Fatal(err)
		}
		value, ok := event.(mcpReply)
		if !ok || value.RequestID != id {
			t.Fatalf("reply %#v", event)
		}
		raw, err := jsonrpc.EncodeMessage(value.Message)
		if err != nil {
			t.Fatal(err)
		}
		return string(raw)
	}
	pass("init", `{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}}`)
	if raw := reply("init"); !strings.Contains(raw, `"tools"`) {
		t.Fatalf("handshake %s", raw)
	}
	ack, err := bridge.pass(ctx, "initialized", []byte(`{"jsonrpc":"2.0","method":"notifications/initialized"}`))
	if err != nil || ack == nil {
		t.Fatalf("notification ack %v %v", ack, err)
	}
	pass("call", `{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mcp__main__read","arguments":{"path":"file"},"_meta":{"claudecode/toolUseId":"tool-1"}}}`)
	event, err := bridge.next(ctx)
	if err != nil {
		t.Fatal(err)
	}
	opened, ok := event.(mcpOpened)
	if !ok || opened.Call.ToolUseID != "tool-1" || opened.Call.ToolName != "read" || string(opened.Call.Input) != `{"path":"file"}` {
		t.Fatalf("opened %#v", event)
	}
	pass("ping", `{"jsonrpc":"2.0","id":3,"method":"ping"}`)
	if raw := reply("ping"); !strings.Contains(raw, `"result"`) {
		t.Fatalf("ping %s", raw)
	}
	pass("list", `{"jsonrpc":"2.0","id":4,"method":"tools/list"}`)
	if raw := reply("list"); !strings.Contains(raw, `"name":"read"`) {
		t.Fatalf("tools %s", raw)
	}
	bridge.deliver("tool-1", toolResult([]provider.ResultPart{provider.TextPart{Text: "done"}, provider.ResultImage{Bytes: provider.MediaBytes{Data: core.NewB64Bytes([]byte("png")), MediaType: "image/png"}}, provider.ResultVideo{Bytes: provider.MediaBytes{Data: core.NewB64Bytes(nil), MediaType: "video/mp4"}}}, true))
	raw := reply("call")
	for _, part := range []string{`"isError":true`, `"text":"done"`, `"data":"cG5n"`, `[video:video/mp4]`} {
		if !strings.Contains(raw, part) {
			t.Fatalf("result %s", raw)
		}
	}
	bridge.deliver("early", toolResult([]provider.ResultPart{provider.TextPart{Text: "ready"}}, false))
	if ids := bridge.unasked(); len(ids) != 1 || ids[0] != "early" {
		t.Fatalf("unasked %v", ids)
	}
	pass("early-call", `{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"read","_meta":{"claudecode/toolUseId":"early"}}}`)
	if raw := reply("early-call"); !strings.Contains(raw, "ready") {
		t.Fatalf("early result %s", raw)
	}
	if len(bridge.unasked()) != 0 {
		t.Fatal("delivered result retained")
	}
	bridge.offer([]provider.ToolDefinition{{Name: "write", InputSchema: jsontext.Value(`{}`)}})
	pass("new-list", `{"jsonrpc":"2.0","id":6,"method":"tools/list"}`)
	if raw := reply("new-list"); !strings.Contains(raw, `"name":"write"`) || strings.Contains(raw, `"name":"read"`) {
		t.Fatalf("changed tools %s", raw)
	}
	pass("bad-call", `{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"write","arguments":[]}}`)
	if raw := reply("bad-call"); !strings.Contains(raw, `"code":-32602`) {
		t.Fatalf("malformed tool call accepted: %s", raw)
	}

	pass("pending-close", `{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"write","_meta":{"claudecode/toolUseId":"pending"}}}`)
	if event, err := bridge.next(ctx); err != nil {
		t.Fatal(err)
	} else if _, ok := event.(mcpOpened); !ok {
		t.Fatalf("pending call: %#v", event)
	}
	closed := make(chan struct{})
	go func() { bridge.close(); close(closed) }()
	select {
	case <-closed:
	case <-ctx.Done():
		t.Fatal("MCP close did not cancel pending calls and join its reader")
	}

	if _, err := bridge.pass(ctx, "after-close", []byte(`{"jsonrpc":"2.0","id":8,"method":"ping"}`)); err == nil {
		t.Fatal("closed MCP bridge accepted another request")
	}

}
