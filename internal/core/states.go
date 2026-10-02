package core

// A message edit, which the web app chooses so that a repeated request
// is recognized.
// +demi:id
// +demi:length chars min=1
// +demi:root
type OperationID string

// A sequence of the numbers the model knows a conversation's things by
// (`runtime.md` § Identifiers the model sees): each is given once, in order,
// across crashes, restores and Forks.
// +demi:enum command shell agent tab
// +demi:root
type Sequence string

// What a session is doing, as clients see it.
// +demi:enum idle running compacting
// +demi:root direction=receive output=protocol
type SessionPhase string

// How a snapshot was filled.
// +demi:enum probe observation
type SnapshotSource string

// The unit of a window's amounts.
// +demi:enum percent credits usd_minor requests tokens
type QuotaUnit string

// How close a window is to its limit.
// +demi:enum normal warning critical
type QuotaSeverity string

// The wire format an `openai` entry speaks (`providers.md` § Endpoints): the
// Responses API, or Chat Completions as OpenAI-compatible vendors serve it.
// An entry that names none speaks Responses.
// +demi:enum responses chat-completions
// +demi:root direction=receive output=protocol
type WireAPI string

// Whether a provider's credential is present and usable. Reading it never
// makes an inference request.
// +demi:root direction=receive output=protocol
// +demi:union tag=status
//
//sumtype:decl
type AuthState interface{ isAuthState() }

// +demi:variant AuthState unknown
// +demi:tolerant
type AuthUnknown struct {
	Message *string `json:"message,omitempty"`
}

// +demi:variant AuthState authenticated
// +demi:tolerant
type Authenticated struct {
	// The account the credential acts for, when the provider can name
	// it.
	AccountLabel *string `json:"accountLabel,omitempty"`
}

// +demi:variant AuthState unauthenticated
// +demi:tolerant
type Unauthenticated struct {
	Message *string `json:"message,omitempty"`
}

// +demi:variant AuthState error
// +demi:tolerant
type AuthError struct {
	Message string `json:"message"`
}

// Whether a provider can run requests at all.
// +demi:root direction=receive output=protocol
// +demi:union tag=status
//
//sumtype:decl
type RuntimeState interface{ isRuntimeState() }

// +demi:variant RuntimeState unknown
// +demi:tolerant
type RuntimeUnknown struct {
	Message *string `json:"message,omitempty"`
}

// +demi:variant RuntimeState ready
// +demi:tolerant
type RuntimeReady struct {
	Message *string `json:"message,omitempty"`
}

// +demi:variant RuntimeState unavailable
// +demi:tolerant
type RuntimeUnavailable struct {
	Message string `json:"message"`
}

// +demi:variant RuntimeState error
// +demi:tolerant
type RuntimeError struct {
	Message string `json:"message"`
}

// Bytes that travel in JSON as a base64 string (RFC 4648, the standard
// alphabet with padding). Cloning shares the bytes.
// +demi:root
// +demi:base64
type B64Bytes []byte
