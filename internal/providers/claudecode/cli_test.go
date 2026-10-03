package claudecode

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"reflect"
	"slices"
	"sync"
	"testing"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

const testToken = "sk-ant-oat01-test-token"

func testClock() core.Clock {
	return providertest.FixedClock(core.Timestamp("2026-09-24T08:00:00.000Z"))
}

func testProvider(t *testing.T, catalog, usage string) (*Provider, *provider.MemoryCredentialPool) {
	t.Helper()
	pool := provider.NewMemoryCredentialPool()
	models := provider.NewModelsDevClient(http.DefaultClient, catalog, testClock())
	staged := New(
		NewConfig("entry-1", "Claude", nil),
		pool,
		&provider.MemorySnapshots{},
		models,
		http.DefaultClient,
		testClock(),
	)
	token, err := provider.NewSecret(testToken)
	if err != nil {
		t.Fatal(err)
	}
	account, err := staged.Accounts().Add(t.Context(), provider.AddAccount{SetupToken: token})
	if err != nil {
		t.Fatal(err)
	}
	config := NewConfig("entry-1", "Claude", &account.ID)
	config.UsageURL = usage
	return New(config, pool, &provider.MemorySnapshots{}, models, http.DefaultClient, testClock()), pool
}

func user(text string) provider.InferenceItem {
	return &provider.UserMessage{Content: []provider.UserPart{&provider.TextPart{Text: text}}}
}

func request(items ...provider.InferenceItem) provider.InferenceRequest {
	return provider.InferenceRequest{
		SessionID:    "session-1",
		ModelID:      "claude-test",
		SystemPrompt: "system",
		Items:        items,
	}
}

func withTools(r provider.InferenceRequest) provider.InferenceRequest {
	r.Tools = []provider.ToolDefinition{
		{
			Name:        "shell_exec",
			Description: "Execute a shell script",
			InputSchema: json.RawMessage(
				`{"type":"object","properties":{"script":{"type":"string"}},` +
					`"required":["script"],"additionalProperties":false}`,
			),
		},
	}
	return r
}

func toolUse(id, script string) provider.InferenceItem {
	return &provider.ToolUse{
		ModelID:   "claude-test",
		ToolUseID: id,
		ToolName:  "shell_exec",
		Input: json.RawMessage(`{"script":"` +
			script +
			`"}`),
	}
}

func toolOutput(id, text string) provider.InferenceItem {
	return &provider.ToolResult{ToolUseID: id, Output: []provider.ResultPart{&provider.TextPart{Text: text}}}
}

func response(input, output uint64) provider.Event {
	return &provider.Response{Usage: core.TokenUsage{InputTokens: input, OutputTokens: output}}
}
func textEvent(text string) provider.Event { return &provider.TextDelta{Text: text} }
func equal(t *testing.T, got, want any) {
	t.Helper()
	if !reflect.DeepEqual(want, got) {
		t.Fatalf("got %#v; want %#v", got, want)
	}
}

func failure(t *testing.T, events []provider.Event) provider.Failure {
	t.Helper()
	if len(events) != 1 {
		t.Fatalf("wanted failure, got %#v", events)
	}
	e, ok := events[0].(*provider.Error)
	if !ok {
		t.Fatalf("wanted failure, got %#v", events[0])
	}
	return e.Failure
}

func collect(ctx context.Context, r provider.Runtime, req provider.InferenceRequest) []provider.Event {
	return slices.Collect(r.Run(ctx, req))
}

// scriptedCLI plays the Claude Code CLI process: it answers what the provider writes with scripted stream-json lines.
// IO waits for channel events, never wall time. No real CLI or model runs.
type scriptedCLI struct {
	t                  *testing.T
	spawn              host.SpawnRequest
	output             chan host.ProcessOutput
	ended              chan struct{}
	mu                 sync.Mutex
	end                host.ProcessEnd
	finished           bool
	signals            []host.Signal
	closed             bool
	closedWhileRunning bool
	ignoreTerminate    bool
	input              []json.RawMessage
	onWrite            func(map[string]json.RawMessage)
}

func (c *scriptedCLI) say(raw string) {
	c.output <- host.ProcessOutput{Stream: core.StreamKind("stdout"), Bytes: []byte(raw + "\n")}
}

func (c *scriptedCLI) sayValue(value any) {
	data, err := provider.JSONBody(value)
	if err != nil {
		c.t.Fatal(err)
	}
	c.say(string(data))
}

func (c *scriptedCLI) text(text string) {
	c.sayValue(
		map[string]any{
			"type": "stream_event",
			"event": map[string]any{
				"type":  "content_block_delta",
				"delta": map[string]any{"type": "text_delta", "text": text},
			},
		},
	)
}

func (c *scriptedCLI) result(input, output int) {
	c.sayValue(
		map[string]any{"type": "result", "usage": map[string]int{"input_tokens": input, "output_tokens": output}},
	)
}

func (c *scriptedCLI) finish(end host.ProcessEnd) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if !c.finished {
		c.end = end
		c.finished = true
		close(c.ended)
		close(c.output)
	}
}

func (c *scriptedCLI) Next(ctx context.Context) (host.ProcessOutput, error) {
	select {
	case output, ok := <-c.output:
		if !ok {
			return host.ProcessOutput{}, io.EOF
		}
		return output, nil
	case <-ctx.Done():
		return host.ProcessOutput{}, ctx.Err()
	}
}

func (c *scriptedCLI) wait(ctx context.Context) (host.ProcessEnd, error) {
	select {
	case <-c.ended:
		c.mu.Lock()
		defer c.mu.Unlock()
		return c.end, nil
	case <-ctx.Done():
		return host.ProcessEnd{}, ctx.Err()
	}
}

func (c *scriptedCLI) WriteStdin(_ context.Context, data []byte) error {
	var value map[string]json.RawMessage
	if err := json.Unmarshal(data, &value); err != nil {
		c.t.Fatal(err)
	}
	c.input = append(c.input, slices.Clone(data))
	if string(value["type"]) == `"control_response"` {
		var outer struct {
			ID string `json:"request_id"`
		}
		if err := json.Unmarshal(value["response"], &outer); err != nil {
			c.t.Fatal(err)
		}
		if outer.ID == "mcp-init" || outer.ID == "mcp-initialized" {
			_, reply := mcpResponse(c.t, value)
			if outer.ID == "mcp-initialized" {
				equal(c.t, decoded(c.t, reply["jsonrpc"]), "2.0")
				equal(c.t, decoded(c.t, reply["id"]), float64(0))
				checkJSON(c.t, reply["result"], `{}`)
				equal(c.t, len(reply), 3)
			} else {
				var result struct {
					Server struct {
						Name string `json:"name"`
					} `json:"serverInfo"`
				}
				if err := json.Unmarshal(reply["result"], &result); err != nil {
					c.t.Fatal(err)
				}
				equal(c.t, result.Server.Name, "demi")
			}
		}
	}
	if c.onWrite != nil {
		c.onWrite(value)
	}
	return nil
}
func (*scriptedCLI) CloseStdin(context.Context) error { return nil }
func (c *scriptedCLI) Kill(_ context.Context, signal host.Signal) error {
	c.signals = append(c.signals, signal)
	if signal != host.Terminate || !c.ignoreTerminate {
		c.finish(host.ProcessEnd{Kind: host.ProcessSignalled, Signal: string(signal)})
	}
	return nil
}

func (c *scriptedCLI) Close(context.Context) error {
	c.mu.Lock()
	c.closedWhileRunning = !c.finished
	c.mu.Unlock()
	c.closed = true
	c.finish(host.ProcessEnd{Kind: host.ProcessSignalled, Signal: "SIGKILL"})
	return nil
}

func (c *scriptedCLI) initialized(v map[string]json.RawMessage) bool {
	if string(v["type"]) != `"control_request"` {
		return false
	}
	var request struct {
		Subtype string `json:"subtype"`
	}
	if err := json.Unmarshal(v["request"], &request); err != nil {
		c.t.Fatal(err)
	}
	equal(c.t, request.Subtype, "initialize")
	c.sayValue(
		map[string]any{
			"type":     "control_response",
			"response": map[string]any{"subtype": "success", "request_id": v["request_id"]},
		},
	)
	return true
}

type scriptedPlacement struct {
	t      *testing.T
	starts []*scriptedCLI
	setup  func(*scriptedCLI)
	fail   error
}

func (p *scriptedPlacement) Start(_ context.Context, spawn func(Site) host.SpawnRequest) (*host.StartedProcess, error) {
	if p.fail != nil {
		return nil, p.fail
	}
	c := &scriptedCLI{
		t:      p.t,
		spawn:  spawn(Site{Executable: "/demi/claude", RunDir: "/demi/run", ConfigDir: "/demi/config"}),
		output: make(chan host.ProcessOutput, 128),
		ended:  make(chan struct{}),
	}
	p.starts = append(p.starts, c)
	if p.setup != nil {
		p.setup(c)
	}
	return &host.StartedProcess{Output: c, Control: c, Wait: c.wait}, nil
}

func fixture(t *testing.T, setup func(*scriptedCLI)) (provider.Runtime, *scriptedPlacement) {
	t.Helper()
	p, _ := testProvider(t, "http://127.0.0.1:9/catalog", "http://127.0.0.1:9/usage")
	placement := &scriptedPlacement{t: t, setup: setup}
	r := p.ProcessRuntime(placement)
	t.Cleanup(func() {
		if err := r.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return r, placement
}

func answer(t *testing.T, lines ...string) ([]provider.Event, *scriptedCLI) {
	t.Helper()
	r, p := fixture(t, func(c *scriptedCLI) {
		c.onWrite = func(map[string]json.RawMessage) {
			for _, line := range lines {
				c.say(line)
			}
		}
	})
	return collect(t.Context(), r, request(user("hi"))), p.starts[0]
}

const (
	doneLine  = `{"type":"result","usage":{"input_tokens":1,"output_tokens":1}}`
	startLine = `{"type":"stream_event","event":{"type":"message_start","message":{}}}`
	stopLine  = `{"type":"stream_event","event":{"type":"message_stop"}}`
)

func toolLine(id, script string) string {
	data, _ := provider.JSONBody(
		map[string]any{
			"type": "assistant",
			"message": map[string]any{
				"content": []any{
					map[string]any{
						"type":  "tool_use",
						"id":    id,
						"name":  "mcp__main__shell_exec",
						"input": map[string]any{"script": script},
					},
				},
			},
		},
	)
	return string(data)
}

func (c *scriptedCLI) mcp(control string, value any) {
	c.sayValue(
		map[string]any{
			"type":       "control_request",
			"request_id": control,
			"request":    map[string]any{"subtype": "mcp_message", "server_name": "main", "message": value},
		},
	)
}

func (c *scriptedCLI) call(control string, id int, toolID, script string) {
	c.mcp(
		control,
		map[string]any{
			"jsonrpc": "2.0",
			"id":      id,
			"method":  "tools/call",
			"params": map[string]any{
				"name":      "shell_exec",
				"arguments": map[string]any{"script": script},
				"_meta":     map[string]any{"claudecode/toolUseId": toolID},
			},
		},
	)
}

func (c *scriptedCLI) handshake() {
	c.mcp(
		"mcp-init",
		map[string]any{
			"jsonrpc": "2.0",
			"id":      0,
			"method":  "initialize",
			"params": map[string]any{
				"protocolVersion": "2025-06-18",
				"capabilities":    map[string]any{},
				"clientInfo":      map[string]any{"name": "claude-code", "version": "2.1.3"},
			},
		},
	)
	c.mcp("mcp-initialized", map[string]any{"jsonrpc": "2.0", "method": "notifications/initialized"})
}

func decoded(t *testing.T, raw []byte) any {
	t.Helper()
	var value any
	if err := json.Unmarshal(raw, &value); err != nil {
		t.Fatal(err)
	}
	return value
}

func checkJSON(t *testing.T, raw []byte, want string) {
	t.Helper()
	equal(t, decoded(t, raw), decoded(t, []byte(want)))
}

func mcpResponse(t *testing.T, v map[string]json.RawMessage) (string, map[string]json.RawMessage) {
	t.Helper()
	equal(t, decoded(t, v["type"]), "control_response")
	var response struct {
		ID       string `json:"request_id"`
		Subtype  string `json:"subtype"`
		Response struct {
			MCP map[string]json.RawMessage `json:"mcp_response"`
		} `json:"response"`
	}
	if err := json.Unmarshal(v["response"], &response); err != nil {
		t.Fatal(err)
	}
	equal(t, response.Subtype, "success")
	return response.ID, response.Response.MCP
}
