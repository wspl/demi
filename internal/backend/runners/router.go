package runners

import (
	"context"
	"fmt"
	"sync"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

// CommandRouter holds the registered nodes of one user's shard. The zero value
// is ready to use; its methods synchronize concurrent callers. Do not copy it.
type CommandRouter struct {
	mu    sync.Mutex // Protects node registrations and their hold counts.
	nodes map[string]*commandNode
}

type commandNode struct {
	id           string
	conversation webapi.ConversationID
	commands     *host.CommandSet
	selection    *remotehost.CommandSelection
	holds        int
}

// Register registers node's commands and manifest for conversation. An existing
// registration is shared. Each returned hold must be released by its environment.
func (r *CommandRouter) Register(node string, conversation webapi.ConversationID, commands *host.CommandSet, selection *remotehost.CommandSelection) *CommandRegistration {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.nodes == nil {
		r.nodes = make(map[string]*commandNode)
	}
	registered := r.nodes[node]
	if registered == nil {
		registered = &commandNode{id: node, conversation: conversation, commands: commands, selection: selection}
		r.nodes[node] = registered
	}
	registered.holds++
	return &CommandRegistration{router: r, node: registered}
}

// SelectionOf returns node's manifest when registered for conversation, else nil.
func (r *CommandRouter) SelectionOf(node string, conversation webapi.ConversationID) *remotehost.CommandSelection {
	r.mu.Lock()
	defer r.mu.Unlock()
	registered := r.nodes[node]
	if registered == nil || registered.conversation != conversation {
		return nil
	}
	return registered.selection
}

// Dispatch runs job's call in its node's commands, refusing a missing node or a
// node registered to another conversation.
func (r *CommandRouter) Dispatch(ctx context.Context, job remotehost.JobOrigin, invocation host.RPCInvocation, port host.RPCPort) (uint8, error) {
	if job.Caller == nil {
		return 0, &host.RPCError{Kind: host.HandlerFailed, Message: "rpc commands run for an agent's jobs"}
	}
	node := string(job.Caller.Node)
	r.mu.Lock()
	registered := r.nodes[node]
	r.mu.Unlock()
	if registered == nil {
		return 0, &host.RPCError{Kind: host.HandlerFailed, Message: fmt.Sprintf("no agent session behind node %s", node)}
	}
	if string(registered.conversation) != job.Context.Conversation {
		return 0, &host.RPCError{Kind: host.HandlerFailed, Message: fmt.Sprintf("node %s belongs to another conversation", node)}
	}
	return registered.commands.Dispatch(ctx, invocation, port)
}

// CommandRegistration is a node's hold on its registration. Do not copy it.
// Its environment defers Release; the last hold removes the registration.
type CommandRegistration struct {
	router   *CommandRouter
	node     *commandNode
	released bool
}

// Retain returns another independently released hold on this registration.
func (r *CommandRegistration) Retain() *CommandRegistration {
	r.router.mu.Lock()
	defer r.router.mu.Unlock()
	if r.released {
		return nil
	}
	r.node.holds++
	return &CommandRegistration{router: r.router, node: r.node}
}

// Release ends this hold exactly once.
func (r *CommandRegistration) Release() {
	r.router.mu.Lock()
	defer r.router.mu.Unlock()
	if r.released {
		return
	}
	r.released = true
	r.node.holds--
	if r.node.holds == 0 && r.router.nodes[r.node.id] == r.node {
		delete(r.router.nodes, r.node.id)
	}
}
