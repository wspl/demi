// Package builtincommands is the demi.builtin command package's handler:
// the operations of [builtinproto], served over a command service. It routes an
// invocation by its operation. The file operations (file.read, file.create,
// file.edit, file.patch) live here; the browser's arrive with the conversation
// browser.
//
// A file operation answers what the agent reads: the file's bytes for a read,
// one line for a mutation, and for a failure the reason, worded as the Rust
// implementation words it (docs/execution/commands.md § File commands).
package builtincommands

import (
	"errors"
	"fmt"

	"github.com/wspl/demi/go/builtinproto"
	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/gates"
)

// A Handler serves the package's operations. It holds no conversation state, so
// the SDK's empty conversation status and its no-op close stand.
type Handler struct {
	// mutations makes file mutations run one at a time.
	mutations *gates.SerialGate
}

// New returns a handler that serves the package's operations.
func New() *Handler {
	return &Handler{mutations: gates.NewSerialGate()}
}

// Operations returns the operations the handler lists: every operation of the
// package.
func (h *Handler) Operations() []string {
	return builtinproto.Operations()
}

// Invoke runs one invocation: it decodes the operation's arguments and runs
// it. An argument that breaks its type's rules is a failure like any other.
func (h *Handler) Invoke(call *commandservice.Call) (commandservice.Completion, error) {
	input, err := builtinproto.Parse(call.Invocation.Operation, call.Invocation.Args)
	if err != nil {
		return commandservice.Completion{}, err
	}
	switch input := input.(type) {
	case builtinproto.ReadArgs:
		err = h.read(call, input)
	case builtinproto.CreateArgs:
		err = h.create(call, input)
	case builtinproto.EditArgs:
		err = h.edit(call, input)
	case builtinproto.PatchArgs:
		err = h.patch(call, input)
	default:
		err = fmt.Errorf("%s is served by the conversation browser, which this program does not have yet", call.Invocation.Operation)
	}
	if errors.Is(err, errCancelled) {
		return commandservice.Completion{}, commandservice.ErrCancelled
	}
	return commandservice.Completion{}, err
}
