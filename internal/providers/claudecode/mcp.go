package claudecode

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strconv"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/provider"
)

// mcpServer is the process's in-memory MCP endpoint. Pending calls are entries,
// not blocked goroutines, so ping and tools/list stay available between calls.
type mcpServer struct {
	tools    []provider.ToolDefinition
	open     map[string]mcpRequest
	ready    map[string]setModerner
	replies  []mcpReply
	requests map[string][]string
	phase    mcpPhase
	failure  string
}
type mcpRequest struct {
	control string
	id      json.RawMessage
	routed  bool
	modern  bool
}
type mcpReply struct {
	request mcpRequest
	result  setModerner
	failure *mcpError
}

func newMCP(tools []provider.ToolDefinition) *mcpServer {
	return &mcpServer{
		tools:    tools,
		open:     make(map[string]mcpRequest),
		ready:    make(map[string]setModerner),
		requests: make(map[string][]string),
	}
}

func toolResult(r *provider.ToolResult) *callResult {
	content := make([]mcpContent, 0, len(r.Output))
	for _, part := range r.Output {
		switch p := part.(type) {
		case *provider.TextPart:
			content = append(content, textBlock(p.Text))
		case *provider.ResultImage:
			content = append(
				content,
				mcpImage{"image", base64.StdEncoding.EncodeToString(p.Bytes.Data), p.Bytes.MediaType},
			)
		case *provider.ResultVideo:
			content = append(content, textBlock("[video:"+p.Bytes.MediaType+"]"))
		}
	}
	return &callResult{Content: content, IsError: r.IsError}
}

func (m *mcpServer) deliver(id string, result setModerner) {
	if pending, ok := m.open[id]; ok {
		delete(m.open, id)
		m.replies = append(m.replies, mcpReply{request: pending, result: result})
	} else {
		m.ready[id] = result
	}
}

func (l *live) flushReplies(ctx context.Context) error {
	if l.mcp == nil {
		return nil
	}
	for len(l.mcp.replies) > 0 {
		reply := l.mcp.replies[0]
		l.mcp.replies[0] = mcpReply{}
		l.mcp.replies = l.mcp.replies[1:]
		if err := l.reply(ctx, reply); err != nil {
			return err
		}
	}
	return nil
}

func (l *live) reply(ctx context.Context, r mcpReply) error {
	if r.request.routed {
		key := string(r.request.id)
		waiting := l.mcp.requests[key]
		if len(waiting) == 0 {
			return nil
		}
		r.request.control = waiting[0]
		if len(waiting) == 1 {
			delete(l.mcp.requests, key)
		} else {
			l.mcp.requests[key] = waiting[1:]
		}
	}
	var value rpcMessage
	if r.failure != nil {
		value = rpcError{"2.0", r.request.id, r.failure}
	} else {
		r.result.setModern(r.request.modern)
		value = rpcSuccess{"2.0", r.request.id, r.result}
	}
	return l.write(
		ctx,
		responseInput{"control_response", controlSuccess{"success", r.request.control, mcpReplyBody{value}}},
	)
}

func (l *live) control(ctx context.Context, line controlLine) (*provider.ToolCall, error) {
	refuse := func(message string) (*provider.ToolCall, error) {
		return nil, l.write(ctx, responseInput{"control_response", controlError{"error", line.ID, message}})
	}
	r := line.Request
	if r.Subtype != "mcp_message" {
		return refuse("Demi does not serve the control request " + r.Subtype)
	}
	if l.mcp == nil || r.Server == nil || *r.Server != "main" || len(r.Message) == 0 || string(r.Message) == "null" {
		name := "(unnamed)"
		if r.Server != nil {
			name = *r.Server
		}
		return refuse("No SDK MCP server " + name + " serves this process")
	}
	if l.mcp.phase == mcpEnded {
		return refuse(l.mcp.failure)
	}
	message, err := provider.DecodeUntagged[mcpMessage](string(r.Message))
	if err != nil {
		return refuse("The SDK MCP message cannot be read: " + err.Error())
	}
	if message.Version != "2.0" {
		return refuse("The SDK MCP message cannot be read: expected JSON-RPC 2.0")
	}
	if message.Method == nil && len(message.Result) == 0 && message.Error == nil {
		return refuse(`The SDK MCP message cannot be read: expected a request, ` +
			`notification, response or error`)
	}
	if len(message.Result) > 0 && (len(message.ID) == 0 || string(message.ID) == "null") {
		return refuse("The SDK MCP message cannot be read: response id is missing")
	}
	if failure := normalizeMCPID(&message); failure != "" {
		return refuse(failure)
	}
	pending := mcpRequest{control: line.ID, id: message.ID}
	if len(message.ID) == 0 || message.Method == nil {
		return l.acknowledgeMCP(ctx, message, pending)
	}
	if string(message.ID) == "null" {
		return refuse("The SDK MCP message cannot be read: invalid request id")
	}
	pending.routed = true
	l.mcp.requests[string(pending.id)] = append(l.mcp.requests[string(pending.id)], line.ID)
	return l.dispatchMCP(ctx, message, pending)
}

type mcpMessage struct {
	Version string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id"      wire:"optional"`
	Method  *string         `json:"method"`
	Params  json.RawMessage `json:"params"  wire:"optional"`
	Result  json.RawMessage `json:"result"  wire:"optional"`
	Error   *struct {
		Code    int64           `json:"code"`
		Message string          `json:"message"`
		Data    json.RawMessage `json:"data" wire:"optional"`
	} `json:"error"`
}

func (l *live) dispatchMCP(ctx context.Context, message mcpMessage, pending mcpRequest) (*provider.ToolCall, error) {
	var result setModerner = &emptyResult{}
	unknown := func() (*provider.ToolCall, error) {
		return nil, l.reply(ctx, mcpReply{request: pending, failure: &mcpError{Code: -32601, Message: *message.Method}})
	}
	params, initErr := provider.DecodeUntagged[initializeParams](string(message.Params))
	isInitialize := *message.Method == "initialize" && initErr == nil
	modern, protocolFailure := l.mcp.admit(*message.Method, message.Params, isInitialize)
	pending.modern = modern
	if protocolFailure != nil {
		return nil, l.reply(ctx, mcpReply{request: pending, failure: protocolFailure})
	}
	switch *message.Method {
	case "initialize":
		if !isInitialize {
			return unknown()
		}
		result = l.mcp.initialize(params)
	case "ping":
		if modern || l.mcp.phase == mcpInline {
			return unknown()
		}
	case "discover":
		result = &discoverResult{"complete", mcpVersions, mcpCapabilities{}, 0, "private", serverMetadata{mcpIdentity}}
	case "tools/list":
		result = l.listMCPTools(message.Params)
		if result == nil {
			return unknown()
		}
	case "tools/call":
		return l.callMCP(ctx, message.Params, pending, unknown)
	case "resources/list", "resources/templates/list", "prompts/list":
		var ok bool
		result, ok = emptyCatalog(*message.Method, message.Params)
		if !ok {
			return unknown()
		}
	case "completion/complete":
		var ok bool
		result, ok = completion(message.Params)
		if !ok {
			return unknown()
		}
	default:
		return unknown()
	}
	return nil, l.reply(ctx, mcpReply{request: pending, result: result})
}

func (l *live) callMCP(
	ctx context.Context,
	raw json.RawMessage,
	pending mcpRequest,
	unknown func() (*provider.ToolCall, error),
) (*provider.ToolCall, error) {
	params, e := provider.DecodeUntagged[struct {
		Name      string                      `json:"name"`
		Arguments json.RawMessage             `json:"arguments" wire:"optional"`
		Meta      *map[string]json.RawMessage `json:"_meta"`
	}](string(raw))
	if e != nil {
		return unknown()
	}
	args := params.Arguments
	if len(args) == 0 || contract.IsNull(args) {
		args = json.RawMessage(`{}`)
	}
	canonical, e := serdeValue(args).MarshalJSON()
	if e != nil {
		return unknown()
	}
	fields, e := contract.ObjectFields(canonical)
	if e != nil {
		return unknown()
	}
	input, e := contract.EncodeObject(fields)
	if e != nil {
		return nil, fmt.Errorf("encode SDK MCP arguments: %w", e)
	}

	toolID := ""
	if params.Meta != nil {
		value := (*params.Meta)["claudecode/toolUseId"]
		if len(value) > 0 {
			toolID, _ = provider.DecodeUntagged[string](string(value))
		}
	}
	if toolID == "" {
		toolID = "mcp-control-" + controlID()
	}
	if ready, ok := l.mcp.ready[toolID]; ok {
		delete(l.mcp.ready, toolID)
		return nil, l.reply(ctx, mcpReply{request: pending, result: ready})
	}

	if previous, ok := l.mcp.open[toolID]; ok {
		l.mcp.replies = append(
			l.mcp.replies,
			mcpReply{
				request: previous,
				failure: &mcpError{Code: -32603, Message: "The tool call ended with its process"},
			},
		)
	}
	l.mcp.open[toolID] = pending
	return &provider.ToolCall{ToolUseID: toolID, ToolName: toolName(params.Name), Input: input}, nil
}

func (l *live) acknowledgeMCP(ctx context.Context, message mcpMessage, pending mcpRequest) (*provider.ToolCall, error) {
	// Nonrequests are acknowledged by the control protocol, including MCP
	// notifications. Cancellation removes its waiting call before acknowledgment.
	if message.Method != nil && *message.Method == "notifications/cancelled" {
		params, e := provider.DecodeUntagged[struct {
			ID json.RawMessage `json:"requestId"`
		}](string(message.Params))
		if e == nil {
			for id, open := range l.mcp.open {
				if string(open.id) == string(params.ID) {
					delete(l.mcp.open, id)
					l.mcp.replies = append(
						l.mcp.replies,
						mcpReply{
							request: open,
							failure: &mcpError{Code: -32603, Message: "The tool call was cancelled"},
						},
					)
				}
			}
		}
	}
	if l.mcp.phase == mcpInitial {
		l.mcp.phase = mcpEnded
		l.mcp.failure = "expect initialized request, but received a notification or response"
	}
	pending.id = json.RawMessage(`0`)
	return nil, l.reply(ctx, mcpReply{request: pending, result: &emptyResult{}})
}

// normalizeMCPID canonicalizes reply routing IDs or returns the control protocol refusal text.
func normalizeMCPID(message *mcpMessage) string {
	if len(message.ID) > 0 && string(message.ID) != "null" {
		id, err := provider.DecodeUntagged[any](string(message.ID))
		if err != nil {
			return "The SDK MCP message cannot be read: " + err.Error()
		}
		switch id := id.(type) {
		case string:
		case json.Number:
			if _, err := strconv.ParseInt(string(id), 10, 64); err != nil {
				return "The SDK MCP message cannot be read: Expected an integer"
			}
		default:
			return "The SDK MCP message cannot be read: invalid request id"
		}
		// Validated strings and integers always encode. Equivalent escaped
		// spellings of an id must route through the same reply queue.
		message.ID, _ = provider.JSONBody(id)
	}
	return ""
}

func (l *live) listMCPTools(raw json.RawMessage) setModerner {
	if len(raw) > 0 && string(raw) != "null" {
		if _, err := provider.DecodeUntagged[struct {
			Cursor *string `json:"cursor"`
		}](string(raw)); err != nil {
			return nil
		}
	}
	tools := make([]mcpTool, 0, len(l.mcp.tools))
	for _, tool := range l.mcp.tools {
		tools = append(tools, mcpTool{tool.Name, tool.Description, serdeValue(tool.InputSchema)})
	}
	return &toolsResult{Tools: tools}
}

func (m *mcpServer) initialize(params initializeParams) setModerner {
	m.phase = mcpSession
	negotiated := params.Version
	switch negotiated {
	case "2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25":
	default:
		negotiated = "2025-11-25"
	}
	return &initializeResult{negotiated, mcpCapabilities{}, mcpIdentity}
}
