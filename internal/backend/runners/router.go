package runners

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

// CommandRouter holds the registered nodes of one user's shard. The zero value
// is ready to use; its methods synchronize concurrent callers. Do not copy it.
type CommandRouter struct{}

// Register registers node's commands and manifest for conversation. An existing
// registration is shared. Each returned hold must be released by its environment.
func (r *CommandRouter) Register(node string, conversation webapi.ConversationID, commands *host.CommandSet, selection *remotehost.CommandSelection) *CommandRegistration {
	panic("not written: b-runners")
}

// SelectionOf returns node's manifest when registered for conversation, else nil.
func (r *CommandRouter) SelectionOf(node string, conversation webapi.ConversationID) *remotehost.CommandSelection {
	panic("not written: b-runners")
}

// Dispatch runs job's call in its node's commands, refusing a missing node or a
// node registered to another conversation.
func (r *CommandRouter) Dispatch(ctx context.Context, job remotehost.JobOrigin, invocation host.RPCInvocation, port host.RPCPort) (uint8, error) {
	panic("not written: b-runners")
}

// CommandRegistration is a node's hold on its registration. Do not copy it.
// Its environment defers Release; the last hold removes the registration.
type CommandRegistration struct{}

// Retain returns another independently released hold on this registration.
func (r *CommandRegistration) Retain() *CommandRegistration { panic("not written: b-runners") }

// Release ends this hold exactly once.
func (r *CommandRegistration) Release() { panic("not written: b-runners") }
