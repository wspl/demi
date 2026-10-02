package claudecode

import "encoding/json"

// rpcMessage is the sealed subset of ServerJsonRpcMessage sent by Demi.
type rpcMessage interface{ rpcMessage() }
type rpcSuccess struct {
	Version string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id"`
	Result  mcpResult       `json:"result"`
}

func (rpcSuccess) rpcMessage() {}

type rpcError struct {
	Version string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id,omitempty"`
	Error   *mcpError       `json:"error"`
}

func (rpcError) rpcMessage() {}

type mcpError struct {
	Code    int                 `json:"code"`
	Message string              `json:"message"`
	Data    *unsupportedVersion `json:"data,omitempty"`
}
type unsupportedVersion struct {
	Requested string   `json:"requested"`
	Supported []string `json:"supported"`
}

// mcpResult follows rmcp's ServerResult variants and declaration order.
// Only result variants with a discriminator embed resultKind.
type mcpResult interface{ setModern(bool) }
type resultKind struct {
	ResultType string `json:"resultType,omitempty"`
}

func (r *resultKind) setModern(modern bool) {
	r.ResultType = ""
	if modern {
		r.ResultType = "complete"
	}
}

type emptyResult struct{}

func (*emptyResult) setModern(bool) {}

type mcpCapabilities struct {
	Tools struct{} `json:"tools"`
}
type initializeResult struct {
	Version      string            `json:"protocolVersion"`
	Capabilities mcpCapabilities   `json:"capabilities"`
	Server       mcpImplementation `json:"serverInfo"`
}

func (*initializeResult) setModern(bool) {}

type discoverResult struct {
	ResultType   string          `json:"resultType"`
	Versions     []string        `json:"supportedVersions"`
	Capabilities mcpCapabilities `json:"capabilities"`
	TTL          uint64          `json:"ttlMs"`
	Scope        string          `json:"cacheScope"`
	Meta         serverMetadata  `json:"_meta"`
}

func (*discoverResult) setModern(bool) {}

type serverMetadata struct {
	Server mcpImplementation `json:"io.modelcontextprotocol/serverInfo"`
}
type toolsResult struct {
	resultKind
	Tools []mcpTool `json:"tools"`
}
type mcpTool struct {
	Name        string     `json:"name"`
	Description string     `json:"description"`
	Schema      serdeValue `json:"inputSchema"`
}
type callResult struct {
	resultKind
	Content []mcpContent `json:"content"`
	IsError bool         `json:"isError"`
}
type mcpContent interface{ mcpContent() }

func (inputText) mcpContent() {}

type mcpImage struct {
	Type string `json:"type"`
	Data string `json:"data"`
	MIME string `json:"mimeType"`
}

func (mcpImage) mcpContent() {}

type resourcesResult struct {
	resultKind
	Resources []struct{} `json:"resources"`
}
type templatesResult struct {
	resultKind
	Templates []struct{} `json:"resourceTemplates"`
}
type promptsResult struct {
	resultKind
	Prompts []struct{} `json:"prompts"`
}
type completionResult struct {
	resultKind
	Completion completionInfo `json:"completion"`
}
type completionInfo struct {
	Values []string `json:"values"`
}
