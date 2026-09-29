package commandservice

import (
	"encoding/json/jsontext"
)

// A CommandCaller says who started work: an agent, by its number, or the
// conversation's user.
//
//demi:union tag=kind
type CommandCaller interface {
	commandCaller()
}

// An AgentCaller is an agent, named by its number in the conversation as the
// model knows it.
//
//demi:variant agent
type AgentCaller struct {
	Number uint64 `json:"number"`
}

// A UserCaller is the conversation's user, through a user stream.
//
//demi:variant user
type UserCaller struct{}

func (AgentCaller) commandCaller() {}
func (UserCaller) commandCaller()  {}

// CommandLocale is an IANA time zone and BCP 47 language tags in preference
// order, as the conversation's user's browser last reported them.
//
//demi:wire
type CommandLocale struct {
	TimeZone  string   `json:"timeZone" check:"chars=1..MaxLocaleChars"`
	Languages []string `json:"languages" check:"items=1..MaxLocaleLanguages,each(chars=1..MaxLocaleChars)"`
}

// The limits of a [CommandLocale].
const (
	// MaxLocaleLanguages is the most language tags it carries.
	MaxLocaleLanguages = 16
	// MaxLocaleChars is the most characters of its time zone and of each tag.
	MaxLocaleChars = 64
)

// A CommandContext is what a declared command knows beyond its arguments:
// which conversation it serves, who started it, and the user's locale. The
// backend is its only source; nothing reads it from the environment.
//
//demi:wire
//demi:msgpack
type CommandContext struct {
	// Conversation names the conversation the work belongs to, or the
	// provider entry that work outside any conversation serves.
	Conversation string        `json:"conversation" check:"chars=1..ConversationNameChars,pattern=nameCharacters"`
	Caller       CommandCaller `json:"caller"`
	Locale       CommandLocale `json:"locale"`
}

// An Invocation is the metadata that opens a native command invocation.
//
//demi:wire
type Invocation struct {
	// Operation names the operation to run.
	Operation string `json:"operation" check:"chars=1.."`
	// InvocationID identifies the invocation to its caller.
	InvocationID string         `json:"invocationId" check:"chars=1.."`
	Context      CommandContext `json:"context"`
	// Args are the operation's arguments, a JSON object. The operation
	// decodes and checks them; the wire only knows they are an object.
	Args jsontext.Value `json:"args" check:"func=requireObject"`
	// Cwd is the invocation's working directory.
	Cwd string `json:"cwd" check:"chars=1..,nonul"`
	// Env is the invocation's environment.
	Env map[string]string `json:"env" check:"keys(pattern=envName),each(nonul)"`
	// Edits says where the invocation records the files it edits, when its
	// job records edits.
	Edits *EditContext `json:"edits,omitzero"`
	// JSON says the caller asked for a JSON result.
	JSON *bool `json:"json,omitzero"`
}

// A CommandError says why a command failed, in the command's own words.
//
//demi:wire
type CommandError struct {
	Code    string `json:"code"`
	Message string `json:"message"`
}

// A Completion says how an invocation ended: its exit code, and the error when
// it failed.
//
//demi:wire
type Completion struct {
	ExitCode uint8         `json:"exitCode"`
	Error    *CommandError `json:"error,omitzero"`
}
