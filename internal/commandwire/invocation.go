package commandwire

import "encoding/json"

//go:generate go run github.com/wspl/demi/tools/contractgen

// Who started the work: an agent, by its number in the conversation as the
// model knows it (`runtime.md` § Identifiers the model sees), or the
// conversation's user through a user stream. `User` is a struct variant so
// that unknown fields are refused: serde ignores them for a unit variant of
// an internally tagged enum.
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

// An IANA time zone and BCP 47 language tags in preference order. The
// web app reports it as a user preference (web-api), so its schema is part
// of the web app's contract too.
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

// What a declared command knows beyond its arguments. The backend is its
// only source; nothing reads it from the environment.
// +demi:msgpack
type CommandContext struct {
	// +demi:pattern ^[A-Za-z0-9_-]{1,64}$
	Conversation string        `json:"conversation"`
	Caller       CommandCaller `json:"caller"`
	Locale       CommandLocale `json:"locale"`
}

// The metadata that opens a native command invocation.
// +demi:check validateInvocation
// +demi:root
type Invocation struct {
	// +demi:length chars min=1
	Operation string `json:"operation"`
	// +demi:length chars min=1
	InvocationID string         `json:"invocationId"`
	Context      CommandContext `json:"context"`
	// The operation's arguments, a JSON object.
	Args json.RawMessage `json:"args"`
	// +demi:length chars min=1
	Cwd   string            `json:"cwd"`
	Env   map[string]string `json:"env"`
	Edits *EditContext      `json:"edits,omitempty"`
	JSON  *bool             `json:"json,omitempty"`
}

// Raw CLI metadata from the local command client (`commands.md` § External
// command clients). The client names only its opaque execution context, in
// `args`; the runner finds the command context through it.
// +demi:check validateLocalInvocation
// +demi:root
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

// Why a command failed, in the command's own words.
type CommandError struct {
	Code    string `json:"code"`
	Message string `json:"message"`
}

// How an invocation ended: its exit code, and the error when it failed.
// +demi:root
type Completion struct {
	ExitCode uint8         `json:"exitCode"`
	Error    *CommandError `json:"error,omitempty"`
}

// Release a conversation's resources, or report which conversations hold
// any. `Status` is a struct variant so that unknown fields are refused: serde
// ignores them for a unit variant of an internally tagged enum.
// +demi:union tag=operation
// +demi:root
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

// The conversations that hold resources in a service.
// +demi:check validateConversationStatus
// +demi:root
type ConversationStatus struct {
	Conversations []string `json:"conversations"`
}

// A sequence of the conversation that a native service draws numbers from.
// +demi:enum tab
// +demi:msgpack
type ServiceSequence string

// TabSequence assigns conversation browser tab numbers.
const TabSequence ServiceSequence = "tab"

// The metadata that opens a stream the runner answers a service's requests
// on, the numbers stream or the artifacts stream, which carries nothing.
// +demi:root
type StreamOpen struct{}

// One request for `count` numbers of the conversation's `sequence`; the
// service writes each as one standard output record. `id` is the service's
// own, unique among its requests in flight.
// +demi:root
type NumbersRequest struct {
	ID uint64 `json:"id"`
	// +demi:pattern ^[A-Za-z0-9_-]{1,64}$
	Conversation string          `json:"conversation"`
	Sequence     ServiceSequence `json:"sequence"`
	// +demi:range min=1 max=16
	Count uint32 `json:"count"`
}

// The answer to request `id`, one input chunk: the first of its `count`
// consecutive numbers, or why there are none.
// +demi:check validateNumbersAnswer
// +demi:root
type NumbersAnswer struct {
	ID uint64 `json:"id"`
	// +demi:range min=1
	First *uint64 `json:"first,omitempty"`
	// +demi:length chars min=1
	Error *string `json:"error,omitempty"`
}
