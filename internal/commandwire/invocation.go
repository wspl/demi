package commandwire

import "encoding/json"

// Select contract files explicitly: framing.go contains behavior-only types.
//go:generate go run ../../tools/contractgen -- invocation.go edits.go package.go artifacts.go validation.go

// CommandCaller identifies who started the command.
// +demi:union tag=kind
//
//sumtype:decl
type CommandCaller interface{ commandCaller() }

// AgentCaller identifies an agent by its conversation number.
// +demi:variant CommandCaller agent
type AgentCaller struct {
	Number uint64 `json:"number"`
}

func (*AgentCaller) commandCaller() {}

// UserCaller identifies the conversation's user.
// +demi:variant CommandCaller user
type UserCaller struct{}

func (*UserCaller) commandCaller() {}

// CommandLocale carries the user's time zone and ordered language preferences.
// +demi:root direction=send output=web
type CommandLocale struct {
	// +demi:length chars min=1 max=64
	TimeZone string `json:"timeZone"`
	// +demi:length min=1 max=16
	Languages []LanguageTag `json:"languages"`
}

// LanguageTag is a bounded language preference supplied by the web app.
// +demi:length chars min=1 max=64
type LanguageTag string

// CommandContext is supplied by the backend, never the environment.
type CommandContext struct {
	// +demi:pattern ^[A-Za-z0-9_-]{1,64}$
	Conversation string        `json:"conversation"`
	Caller       CommandCaller `json:"caller"`
	Locale       CommandLocale `json:"locale"`
}

// Invocation opens a native command invocation.
// +demi:check validateInvocation
type Invocation struct {
	// +demi:length chars min=1
	Operation string `json:"operation"`
	// +demi:length chars min=1
	InvocationID string          `json:"invocationId"`
	Context      CommandContext  `json:"context"`
	Args         json.RawMessage `json:"args"`
	// +demi:length chars min=1
	Cwd   string            `json:"cwd"`
	Env   map[string]string `json:"env"`
	Edits *EditContext      `json:"edits,omitempty"`
	JSON  *bool             `json:"json,omitempty"`
}

// LocalInvocation carries raw CLI metadata from the local command client.
// +demi:check validateLocalInvocation
type LocalInvocation struct {
	// +demi:length chars min=1
	Operation string `json:"operation"`
	// +demi:length chars min=1
	InvocationID string          `json:"invocationId"`
	Args         json.RawMessage `json:"args"`
	// +demi:length chars min=1
	Cwd string            `json:"cwd"`
	Env map[string]string `json:"env"`
}

// CommandError explains a command failure.
type CommandError struct {
	Code    string `json:"code"`
	Message string `json:"message"`
}

// Completion is the final status of an invocation.
type Completion struct {
	ExitCode uint8         `json:"exitCode"`
	Error    *CommandError `json:"error,omitempty"`
}

// ConversationRequest releases or queries service-held conversation resources.
// +demi:union tag=operation
//
//sumtype:decl
type ConversationRequest interface{ conversationRequest() }

// ConversationRelease releases one conversation.
// +demi:variant ConversationRequest release
type ConversationRelease struct {
	// +demi:pattern ^[A-Za-z0-9_-]{1,64}$
	Conversation string `json:"conversation"`
}

func (*ConversationRelease) conversationRequest() {}

// ConversationQuery asks which conversations hold resources.
// +demi:variant ConversationRequest status
type ConversationQuery struct{}

func (*ConversationQuery) conversationRequest() {}

// ConversationStatus lists conversations holding resources.
// +demi:check validateConversationStatus
type ConversationStatus struct {
	Conversations []string `json:"conversations"`
}

// ServiceSequence identifies a conversation number sequence.
// +demi:enum tab
type ServiceSequence string

// TabSequence assigns conversation browser tab numbers.
const TabSequence ServiceSequence = "tab"

// StreamOpen opens the numbers or artifacts stream.
type StreamOpen struct{}

// NumbersRequest reserves consecutive conversation numbers.
type NumbersRequest struct {
	ID uint64 `json:"id"`
	// +demi:pattern ^[A-Za-z0-9_-]{1,64}$
	Conversation string          `json:"conversation"`
	Sequence     ServiceSequence `json:"sequence"`
	// +demi:range min=1 max=16
	Count uint32 `json:"count"`
}

// NumbersAnswer carries exactly one of First or Error.
// +demi:check validateNumbersAnswer
type NumbersAnswer struct {
	ID uint64 `json:"id"`
	// +demi:range min=1
	First *uint64 `json:"first,omitempty"`
	// +demi:length chars min=1
	Error *string `json:"error,omitempty"`
}
