package claudecode

// inputLine is the sealed stream-json input union.
type (
	inputLine interface{ inputLine() }
	userInput struct {
		Type    string       `json:"type"`
		Message inputMessage `json:"message"`
	}
)

func (userInput) inputLine() {}

type inputMessage struct {
	Role    string         `json:"role"`
	Content []inputContent `json:"content"`
}
type initializeInput struct {
	Type    string            `json:"type"`
	ID      string            `json:"request_id"`
	Request initializeRequest `json:"request"`
}

func (initializeInput) inputLine() {}

type initializeRequest struct {
	Subtype string   `json:"subtype"`
	Servers []string `json:"sdkMcpServers"`
	Prompt  string   `json:"systemPrompt"`
}
type responseInput struct {
	Type     string          `json:"type"`
	Response controlResponse `json:"response"`
}

func (responseInput) inputLine() {}

type (
	controlResponse interface{ controlResponse() }
	controlSuccess  struct {
		Subtype  string       `json:"subtype"`
		ID       string       `json:"request_id"`
		Response mcpReplyBody `json:"response"`
	}
)

func (controlSuccess) controlResponse() {}

type controlError struct {
	Subtype string `json:"subtype"`
	ID      string `json:"request_id"`
	Error   string `json:"error"`
}

func (controlError) controlResponse() {}

type mcpReplyBody struct {
	Message rpcMessage `json:"mcp_response"`
}

type (
	inputContent interface{ inputContent() }
	inputText    struct {
		Type string `json:"type"`
		Text string `json:"text"`
	}
)

func (inputText) inputContent() {}

type inputImage struct {
	Type   string      `json:"type"`
	Source imageSource `json:"source"`
}

func (inputImage) inputContent() {}

type inputDocument struct {
	Type   string       `json:"type"`
	Source base64Source `json:"source"`
	Title  string       `json:"title"`
}

func (inputDocument) inputContent() {}

type (
	imageSource  interface{ imageSource() }
	base64Source struct {
		Type      string `json:"type"`
		MediaType string `json:"media_type"`
		Data      string `json:"data"`
	}
)

func (base64Source) imageSource() {}

type urlSource struct {
	Type string `json:"type"`
	URL  string `json:"url"`
}

func (urlSource) imageSource() {}
