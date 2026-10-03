package plugin

import (
	"encoding/json"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

//revive:disable:exported
// Contract doc comments describe the wire value; they do not start with the type's name.

// +demi:root
// +demi:union tag=type
//
//sumtype:decl
type Request interface{ request() }

// The model ran an `rpc` leaf of the plugin's commands. The
// invocation's path starts at the plugin's own tree: a group the plugin
// placed under `demi` arrives without the `demi` before it.
// +demi:variant Request command
type RequestCommand struct {
	User       webapi.UserID      `json:"user"`
	Invocation host.RPCInvocation `json:"invocation"`
}

func (*RequestCommand) request() {}

// A node is about to send a provider request, and the plugin, a
// context source, may add a context block (`plugins.md` § Prompt text
// and context): `seen` holds the text of its own blocks the model
// receives, oldest first, and `cwd` is the node's working directory.
// +demi:variant Request context
type RequestContext struct {
	User         webapi.UserID         `json:"user"`
	Conversation webapi.ConversationID `json:"conversation"`
	Node         core.NodeID           `json:"node"`
	CWD          string                `json:"cwd"`
	Turn         core.TurnID           `json:"turn"`
	Seen         []string              `json:"seen"`
}

func (*RequestContext) request() {}

// The plugin's page state: the user's, or, with `conversation`, that
// conversation's.
// +demi:variant Request page_state
type RequestPageState struct {
	User webapi.UserID `json:"user"`
	// +demi:nullable
	Conversation *webapi.ConversationID `json:"conversation,omitempty"`
}

func (*RequestPageState) request() {}

// A page called a method, with parameters that are valid against the
// method's schema; `conversation` is the conversation a method of the
// conversation scope was called for.
// +demi:variant Request page_call
// +demi:check validatePageCall
type RequestPageCall struct {
	User   webapi.UserID   `json:"user"`
	Method string          `json:"method"`
	Params json.RawMessage `json:"params"`
	// +demi:nullable
	Conversation *webapi.ConversationID `json:"conversation,omitempty"`
}

func (*RequestPageCall) request() {}

// +demi:root
// +demi:union tag=type
//
//sumtype:decl
type Reply interface{ reply() }

// A command's exit status.
// +demi:variant Reply exit
type ReplyExit struct {
	Code uint8 `json:"code"`
}

func (*ReplyExit) reply() {}

// A context request's new block; none to add nothing.
// +demi:variant Reply context
type ReplyContext struct {
	// +demi:nullable
	Text *string `json:"text,omitempty"`
}

func (*ReplyContext) reply() {}

// The page state, valid against its scope's declared schema.
// +demi:variant Reply state
type ReplyState struct {
	State json.RawMessage `json:"state"`
}

func (*ReplyState) reply() {}

// A page call's result, valid against the method's result schema.
// +demi:variant Reply result
type ReplyResult struct {
	Result json.RawMessage `json:"result"`
}

func (*ReplyResult) reply() {}

// Why a plugin gave up on a request.
// +demi:root
// +demi:union tag=type
//
//sumtype:decl
type Error interface {
	pluginError()
	error
}

// The request does not fit, such as a command's arguments its input
// refuses.
// +demi:variant Error usage
type ErrorUsage struct {
	Message string `json:"message"`
}

func (*ErrorUsage) pluginError() {}

// The plugin refuses a page call: `reason` is a snake_case word, such
// as `tab_not_found`, which the page sees with the message.
// +demi:variant Error refused
type ErrorRefused struct {
	Reason  string `json:"reason"`
	Message string `json:"message"`
}

func (*ErrorRefused) pluginError() {}

// A port operation was refused, and the plugin passes the refusal on:
// the caller sees the refusal's own answer.
// +demi:variant Error port
type ErrorPort struct {
	Refusal PortRefusal `json:"refusal"`
}

func (*ErrorPort) pluginError() {}

// The plugin failed.
// +demi:variant Error failed
type ErrorFailed struct {
	Message string `json:"message"`
}

func (*ErrorFailed) pluginError() {}

// The request is over: cancelled, or its caller went away.
// +demi:variant Error ended
type ErrorEnded struct {
	Message string `json:"message"`
}

func (*ErrorEnded) pluginError() {}
