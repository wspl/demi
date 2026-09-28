package commandservice

// Endpoint paths for native command protocol version 1.
const (
	// InfoPath returns the service catalog.
	InfoPath = "/v1/info"
	// InvokePath starts a command invocation.
	InvokePath = "/v1/invoke"
	// ConversationPath handles trusted conversation lifecycle requests.
	ConversationPath = "/v1/conversation"
	// NumbersPath opens the connection's numbers stream.
	NumbersPath = "/v1/numbers"
	// ShutdownPath requests a drain.
	ShutdownPath = "/v1/shutdown"
	// HeaderListBytes bounds each HTTP/2 header list on the wire.
	HeaderListBytes = 16384
)

// net/http adds this HTTP/1 allowance when deriving its HTTP/2 header setting.
const httpHeaderAdjustment = 320
