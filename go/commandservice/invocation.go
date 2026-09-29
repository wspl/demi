package commandservice

import (
	"encoding/json/jsontext"
	"fmt"

	"github.com/google/jsonschema-go/jsonschema"
)

// A CallerKind says who started work.
type CallerKind string

// The kinds of caller.
const (
	// CallerAgent is an agent, named by its number in the conversation as the
	// model knows it.
	CallerAgent CallerKind = "agent"
	// CallerUser is the conversation's user, through a user stream.
	CallerUser CallerKind = "user"
)

// A CommandCaller says who started work: an agent, by its number, or the
// conversation's user. Build one with [AgentCaller] or [UserCaller].
type CommandCaller struct {
	Kind CallerKind `json:"kind"`
	// Number is the agent's number; the user has none.
	Number *uint64 `json:"number,omitzero"`
}

// AgentCaller returns the caller that is the agent with the given number.
func AgentCaller(number uint64) CommandCaller {
	return CommandCaller{Kind: CallerAgent, Number: &number}
}

// UserCaller returns the caller that is the conversation's user.
func UserCaller() CommandCaller {
	return CommandCaller{Kind: CallerUser}
}

var callerWire = declare[CommandCaller](func(s *jsonschema.Schema) {
	tagged(s, "kind", string(CallerAgent), string(CallerUser), form{requires: []string{"number"}}, form{refuses: []string{"number"}})
}, nil)

// CommandLocale is an IANA time zone and BCP 47 language tags in preference
// order, as the conversation's user's browser last reported them.
type CommandLocale struct {
	TimeZone  string   `json:"timeZone"`
	Languages []string `json:"languages"`
}

// The limits of a [CommandLocale].
const (
	// MaxLocaleLanguages is the most language tags it carries.
	MaxLocaleLanguages = 16
	// MaxLocaleChars is the most characters of its time zone and of each tag.
	MaxLocaleChars = 64
)

var localeWire = declare[CommandLocale](func(s *jsonschema.Schema) {
	limitLength(prop(s, "timeZone"), 1, MaxLocaleChars)
	languages := prop(s, "languages")
	languages.MinItems = jsonschema.Ptr(1)
	languages.MaxItems = jsonschema.Ptr(MaxLocaleLanguages)
	limitLength(languages.Items, 1, MaxLocaleChars)
}, nil)

// A CommandContext is what a declared command knows beyond its arguments:
// which conversation it serves, who started it, and the user's locale. The
// backend is its only source; nothing reads it from the environment.
type CommandContext struct {
	// Conversation names the conversation the work belongs to, or the
	// provider entry that work outside any conversation serves.
	Conversation string        `json:"conversation"`
	Caller       CommandCaller `json:"caller"`
	Locale       CommandLocale `json:"locale"`
}

var contextWire = declare[CommandContext](func(s *jsonschema.Schema) {
	limitConversationName(prop(s, "conversation"))
}, nil, callerWire, localeWire)

// An Invocation is the metadata that opens a native command invocation.
type Invocation struct {
	// Operation names the operation to run.
	Operation string `json:"operation"`
	// InvocationID identifies the invocation to its caller.
	InvocationID string         `json:"invocationId"`
	Context      CommandContext `json:"context"`
	// Args are the operation's arguments, a JSON object. The operation
	// decodes and checks them; the wire only knows they are an object.
	Args jsontext.Value `json:"args"`
	// Cwd is the invocation's working directory.
	Cwd string `json:"cwd"`
	// Env is the invocation's environment.
	Env map[string]string `json:"env"`
	// Edits says where the invocation records the files it edits, when its
	// job records edits.
	Edits *EditContext `json:"edits,omitzero"`
	// JSON says the caller asked for a JSON result.
	JSON *bool `json:"json,omitzero"`
}

var _ = declare(func(s *jsonschema.Schema) {
	prop(s, "operation").MinLength = jsonschema.Ptr(1)
	prop(s, "invocationId").MinLength = jsonschema.Ptr(1)
	*prop(s, "args") = jsonschema.Schema{Type: "object"}
	prop(s, "cwd").Pattern = patternNonEmptyNoNUL
	env := prop(s, "env")
	env.PropertyNames = &jsonschema.Schema{Pattern: patternEnvName}
	env.AdditionalProperties.Pattern = patternNoNUL
}, func(v *Invocation) error {
	if v.Edits != nil {
		if err := checkEditContext(v.Edits); err != nil {
			return fmt.Errorf("edits: %w", err)
		}
	}
	return nil
}, contextWire, editContextWire)

// A CommandError says why a command failed, in the command's own words.
type CommandError struct {
	Code    string `json:"code"`
	Message string `json:"message"`
}

// A Completion says how an invocation ended: its exit code, and the error when
// it failed.
type Completion struct {
	ExitCode uint8         `json:"exitCode"`
	Error    *CommandError `json:"error,omitzero"`
}

var _ = declare[Completion](nil, nil)
