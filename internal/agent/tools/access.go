package tools

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

// StoreNumbers adapts the conversation's durable command and shell sequences.
type StoreNumbers struct{ Store store.TreeStore }

// Next records the next sequence value before returning the assigned number.
func (n StoreNumbers) Next(ctx context.Context, sequence core.Sequence) (uint64, error) {
	panic("not written: a-tools")
}

// EnvironmentScope describes the node an environment is made for.
type EnvironmentScope struct {
	// Root is the conversation's root node.
	Root core.NodeID
	// Node is the node whose shells the environment owns.
	Node core.NodeID
	// Agent is the node's model-facing agent number.
	Agent uint64
	// Commands are the commands offered by the node's shells.
	Commands *host.CommandSet
	// Feed reports command pages and whether any page watches.
	Feed host.PageFeed
	// Numbers assigns the conversation's durable command and shell numbers.
	Numbers host.Numbers
}

// ShellEnvironmentFactory makes one node's environment on one of the
// product's Hosts. Tools do not know which shell engine runs it.
type ShellEnvironmentFactory[H host.Host] interface {
	Create(ctx context.Context, scope EnvironmentScope, target H) (host.ShellEnvironment, error)
}

// ShellAccess binds a node to the current Host resolver, the product's
// environment factory and the environments already made for that node.
// Its configuration is immutable while calls run; Environments owns cleanup.
type ShellAccess[H host.Host] struct {
	Hosts        HostResolver[H]
	Shells       ShellEnvironmentFactory[H]
	Environments *Environments
	Context      NodeContext
	Agent        uint64
	Commands     *host.CommandSet
	Feed         host.PageFeed
	Numbers      host.Numbers
}

// Invoke runs one standard tool call. Refused input returns an error outcome;
// Host failures return *session.ToolFailure, and cancellation returns ctx.Err().
// The context owns commands started by the call after Invoke returns.
func (s *ShellAccess[H]) Invoke(ctx context.Context, call session.ToolInvocation) (session.ToolOutcome, error) {
	panic("not written: a-tools")
}

// Write writes stdin to a running command of the current Host and returns
// its environment. A refusal or Host failure returns *CallError.
func (s *ShellAccess[H]) Write(ctx context.Context, command core.CommandID, stdin string) (host.ShellEnvironment, error) {
	panic("not written: a-tools")
}

// Abort stops a running command of the current Host and returns its environment.
// A refusal or Host failure returns *CallError.
func (s *ShellAccess[H]) Abort(ctx context.Context, command core.CommandID) (host.ShellEnvironment, error) {
	panic("not written: a-tools")
}

// CallErrorKind distinguishes refused input from a failed Host or shell.
type CallErrorKind uint8

const (
	// Refused means the input was refused with the text the model receives.
	Refused CallErrorKind = iota
	// Failed means the Host or its shells failed.
	Failed
)

// CallError explains why a shell operation produced no tool result.
// Cause preserves an underlying error for errors.Is and errors.As.
type CallError struct {
	Kind    CallErrorKind
	Message string
	Cause   error
}

// Error returns the refusal or failure message.
func (e *CallError) Error() string { panic("not written: a-tools") }

// Unwrap returns the underlying failure, if any.
func (e *CallError) Unwrap() error { panic("not written: a-tools") }
