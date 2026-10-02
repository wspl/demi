package core

// OperationID identifies a retriable message edit.
// +demi:id
// +demi:length chars min=1
type OperationID string

// Sequence names a conversation-wide sequence of model-visible numbers.
// +demi:enum command shell agent tab
type Sequence string

// SessionPhase describes the session activity visible to clients.
// +demi:enum idle running compacting
// +demi:root direction=receive output=protocol
type SessionPhase string

// SnapshotSource identifies how a quota snapshot was filled.
// +demi:enum probe observation
type SnapshotSource string

// QuotaUnit names the unit a quota window meters.
// +demi:enum percent credits usd_minor requests tokens
type QuotaUnit string

// QuotaSeverity describes how close a quota window is to its limit.
// +demi:enum normal warning critical
type QuotaSeverity string

// WireAPI selects Responses or Chat Completions for an OpenAI-shaped entry.
// +demi:enum responses chat-completions
// +demi:root direction=receive output=protocol
type WireAPI string

// AuthState describes whether a provider credential is usable.
// +demi:root direction=receive output=protocol
// +demi:union tag=status
//
//sumtype:decl
type AuthState interface{ isAuthState() }

// AuthUnknown means credential status has not been determined.
// +demi:variant AuthState unknown
// +demi:tolerant
type AuthUnknown struct {
	Message *string `json:"message,omitempty"`
}

// Authenticated identifies a usable credential and optionally its account.
// +demi:variant AuthState authenticated
// +demi:tolerant
type Authenticated struct {
	AccountLabel *string `json:"accountLabel,omitempty"`
}

// Unauthenticated means a usable credential is absent.
// +demi:variant AuthState unauthenticated
// +demi:tolerant
type Unauthenticated struct {
	Message *string `json:"message,omitempty"`
}

// AuthError reports a failed credential check.
// +demi:variant AuthState error
// +demi:tolerant
type AuthError struct {
	Message string `json:"message"`
}

// RuntimeState describes whether a provider can run requests.
// +demi:root direction=receive output=protocol
// +demi:union tag=status
//
//sumtype:decl
type RuntimeState interface{ isRuntimeState() }

// RuntimeUnknown means runtime availability has not been determined.
// +demi:variant RuntimeState unknown
// +demi:tolerant
type RuntimeUnknown struct {
	Message *string `json:"message,omitempty"`
}

// RuntimeReady means the provider can run requests.
// +demi:variant RuntimeState ready
// +demi:tolerant
type RuntimeReady struct {
	Message *string `json:"message,omitempty"`
}

// RuntimeUnavailable explains why the provider cannot run.
// +demi:variant RuntimeState unavailable
// +demi:tolerant
type RuntimeUnavailable struct {
	Message string `json:"message"`
}

// RuntimeError reports a failed runtime check.
// +demi:variant RuntimeState error
// +demi:tolerant
type RuntimeError struct {
	Message string `json:"message"`
}

// B64Bytes carries padded standard base64 in JSON.
type B64Bytes []byte
