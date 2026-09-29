package claudecode

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"slices"
	"sync"

	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
	"github.com/modelcontextprotocol/go-sdk/mcp"
	"github.com/wspl/demi/go/provider"
)

type mcpEvent interface{ mcpEvent() }
type mcpReply struct {
	RequestID string
	Message   jsonrpc.Message
}

func (mcpReply) mcpEvent() {}

type mcpOpened struct{ Call provider.ToolCall }

func (mcpOpened) mcpEvent() {}

// cliMCP owns the SDK session and its two input paths: vendor JSON-RPC messages
// and the agent's tool results. Close cancels handlers and joins the reply reader.
type cliMCP struct {
	mu       sync.Mutex
	tools    []provider.ToolDefinition
	open     map[string]chan *mcp.CallToolResult
	ready    map[string]*mcp.CallToolResult
	requests map[jsonrpc.ID][]string
	peer     mcp.Connection
	session  *mcp.ServerSession
	events   chan mcpEvent
	cancel   context.CancelFunc
	done     chan struct{}
}

func startMCP(ctx context.Context, tools []provider.ToolDefinition) (*cliMCP, error) {
	ctx, cancel := context.WithCancel(ctx)
	b := &cliMCP{tools: slices.Clone(tools), open: map[string]chan *mcp.CallToolResult{}, ready: map[string]*mcp.CallToolResult{}, requests: map[jsonrpc.ID][]string{}, events: make(chan mcpEvent), cancel: cancel, done: make(chan struct{})}
	server := mcp.NewServer(&mcp.Implementation{Name: "demi", Version: "0.1.3"}, &mcp.ServerOptions{Capabilities: &mcp.ServerCapabilities{Tools: &mcp.ToolCapabilities{}}})
	// The generic SDK tool registry enforces object input schemas and a known
	// tool name. The CLI bridge must forward the agent's current tool catalog
	// and calls as received, so its handlers use the SDK's method middleware.
	server.AddReceivingMiddleware(func(next mcp.MethodHandler) mcp.MethodHandler {
		return func(ctx context.Context, method string, request mcp.Request) (mcp.Result, error) {
			switch method {
			case "tools/list":
				b.mu.Lock()
				tools := slices.Clone(b.tools)
				b.mu.Unlock()
				list := make([]*mcp.Tool, 0, len(tools))
				for _, tool := range tools {
					list = append(list, &mcp.Tool{Name: tool.Name, Description: tool.Description, InputSchema: json.RawMessage(tool.InputSchema)})
				}
				return &mcp.ListToolsResult{Tools: list}, nil
			case "tools/call":
				call, ok := request.(*mcp.CallToolRequest)
				if !ok {
					return nil, &jsonrpc.Error{Code: jsonrpc.CodeInvalidParams, Message: "invalid tool call"}
				}
				return b.call(ctx, call)
			default:
				return next(ctx, method, request)
			}
		}
	})
	// Match the CLI line bound; the SDK's default in-memory transport is only 16 MiB.
	left, right := net.Pipe()
	serverTransport := &mcp.IOTransport{Reader: left, Writer: left, MaxLineLength: 64 << 20}
	clientTransport := &mcp.IOTransport{Reader: right, Writer: right, MaxLineLength: 64 << 20}
	session, err := server.Connect(ctx, serverTransport, nil)
	if err != nil {
		cancel()
		left.Close()
		right.Close()
		return nil, err
	}
	peer, err := clientTransport.Connect(ctx)
	if err != nil {
		cancel()
		session.Close()
		return nil, err
	}
	b.session, b.peer = session, peer
	go func() {
		defer close(b.done)
		for {
			message, err := peer.Read(ctx)
			if err != nil {
				return
			}
			reply, ok := message.(*jsonrpc.Response)
			if !ok {
				continue
			}
			b.mu.Lock()
			waiting := b.requests[reply.ID]
			requestID := ""
			if len(waiting) > 0 {
				requestID = waiting[0]
				if len(waiting) == 1 {
					delete(b.requests, reply.ID)
				} else {
					b.requests[reply.ID] = waiting[1:]
				}
			}
			b.mu.Unlock()
			if requestID != "" {
				select {
				case b.events <- mcpReply{RequestID: requestID, Message: message}:
				case <-ctx.Done():
					return
				}
			}
		}
	}()
	return b, nil
}
func (b *cliMCP) close() {
	b.cancel()
	b.peer.Close()
	b.session.Close()
	<-b.done
}
func (b *cliMCP) offer(tools []provider.ToolDefinition) {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.tools = slices.Clone(tools)
}
func (b *cliMCP) pass(ctx context.Context, requestID string, raw []byte) (jsonrpc.Message, error) {
	message, err := jsonrpc.DecodeMessage(raw)
	if err != nil {
		return nil, err
	}
	request, isRequest := message.(*jsonrpc.Request)
	isCall := isRequest && request.IsCall()
	if isCall {
		b.mu.Lock()
		b.requests[request.ID] = append(b.requests[request.ID], requestID)
		b.mu.Unlock()
	}
	if err := b.peer.Write(ctx, message); err != nil {
		return nil, err
	}
	if isCall {
		return nil, nil
	}
	id, _ := jsonrpc.MakeID(float64(0))
	return &jsonrpc.Response{ID: id, Result: json.RawMessage(`{}`)}, nil
}
func (b *cliMCP) next(ctx context.Context) (mcpEvent, error) {
	for {
		select {
		case event := <-b.events:
			if opened, ok := event.(mcpOpened); ok {
				b.mu.Lock()
				_, stillOpen := b.open[opened.Call.ToolUseID]
				b.mu.Unlock()
				if !stillOpen {
					continue
				}
			}
			return event, nil
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
}
func (b *cliMCP) call(ctx context.Context, request *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
	id, _ := request.Params.Meta["claudecode/toolUseId"].(string)
	if id == "" {
		id = provider.NewToolUseID("mcp-control-")
	}
	input := request.Params.Arguments
	if len(input) == 0 || string(input) == "null" {
		input = json.RawMessage(`{}`)
	}
	var object map[string]json.RawMessage
	if err := json.Unmarshal(input, &object); err != nil || object == nil {
		return nil, &jsonrpc.Error{Code: jsonrpc.CodeInvalidParams, Message: "tool arguments must be an object"}
	}
	call := provider.ToolCall{ToolUseID: id, ToolName: toolName(request.Params.Name), Input: append([]byte(nil), input...)}
	b.mu.Lock()
	if result := b.ready[id]; result != nil {
		delete(b.ready, id)
		b.mu.Unlock()
		return result, nil
	}
	result := make(chan *mcp.CallToolResult, 1)
	previous := b.open[id]
	b.open[id] = result
	b.mu.Unlock()
	if previous != nil {
		previous <- nil
	}
	defer func() {
		b.mu.Lock()
		if b.open[id] == result {
			delete(b.open, id)
		}
		b.mu.Unlock()
	}()
	select {
	case b.events <- mcpOpened{Call: call}:
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	select {
	case answer := <-result:
		if answer == nil {
			return nil, errors.New("The tool call ended with its process")
		}
		return answer, nil
	case <-ctx.Done():
		return nil, errors.New("The tool call was cancelled")
	}
}
func (b *cliMCP) deliver(id string, result *mcp.CallToolResult) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if waiter := b.open[id]; waiter != nil {
		delete(b.open, id)
		waiter <- result
	} else {
		b.ready[id] = result
	}
}
func (b *cliMCP) unasked() []string {
	b.mu.Lock()
	defer b.mu.Unlock()
	ids := make([]string, 0, len(b.ready))
	for id := range b.ready {
		ids = append(ids, id)
	}
	slices.Sort(ids)
	return ids
}
func toolResult(output []provider.ResultPart, isError bool) *mcp.CallToolResult {
	content := make([]mcp.Content, 0, len(output))
	for _, part := range output {
		switch part := part.(type) {
		case provider.TextPart:
			content = append(content, &mcp.TextContent{Text: part.Text})
		case provider.ResultImage:
			content = append(content, &mcp.ImageContent{Data: part.Bytes.Data.Bytes(), MIMEType: part.Bytes.MediaType})
		case provider.ResultVideo:
			content = append(content, &mcp.TextContent{Text: fmt.Sprintf("[video:%s]", part.Bytes.MediaType)})
		}
	}
	return &mcp.CallToolResult{Content: content, IsError: isError}
}
