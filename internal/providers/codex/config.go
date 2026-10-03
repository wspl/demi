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
// A zero field takes the product's value.
type Config struct {
	// The account the provider stands for; nil only for a provider built
	// to log in, which has no account yet.
	Account *string
	// The ChatGPT backend; empty for https://chatgpt.com/backend-api.
	BackendURL string
	// The sign-in service; empty for https://auth.openai.com.
	AuthURL   string
	Transport TransportMode
	// How long a server-sent events request waits for its response headers;
	// zero for 20 seconds.
	HeaderTimeout time.Duration
	// How long a WebSocket waits to connect; zero for 10 seconds.
	ConnectTimeout time.Duration
	// How long a WebSocket may go without a message; zero for no limit.
	StreamIdleTimeout time.Duration
}
