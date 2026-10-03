package codex

import "time"

// TransportMode selects how a request reaches the Responses backend.
type TransportMode uint8

const (
	// Auto tries a WebSocket, falling back before its first event.
	Auto TransportMode = iota
	// SSE uses server-sent events.
	SSE
	// WebSocket uses only a WebSocket.
	WebSocket
)

// Config is the configuration of a codex entry's provider for one account.
type Config struct {
	// The account the provider stands for; nil only for a provider built
	// to log in, which has no account yet.
	Account *string
	// The ChatGPT backend, https://chatgpt.com/backend-api in the product.
	BackendURL string
	// The sign-in service, https://auth.openai.com in the product.
	AuthURL   string
	Transport TransportMode
	// How long a server-sent events request waits for its response headers.
	HeaderTimeout time.Duration
	// How long a WebSocket waits to connect.
	ConnectTimeout time.Duration
	// How long a WebSocket may go without a message; nil for no limit.
	StreamIdleTimeout *time.Duration
}

// NewConfig returns the product's configuration for account.
func NewConfig(account *string) Config {
	return Config{
		Account:        account,
		BackendURL:     "https://chatgpt.com/backend-api",
		AuthURL:        "https://auth.openai.com",
		HeaderTimeout:  20 * time.Second,
		ConnectTimeout: 10 * time.Second,
	}
}
