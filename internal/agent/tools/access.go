package tools

import (
	"context"
	"strconv"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

// StoreNumbers adapts the conversation's durable command and shell sequences.
type StoreNumbers struct {
	// Store owns the conversation's durable sequences.
	Store store.Tree
}

// Next records the next sequence value before returning the assigned number.
func (n StoreNumbers) Next(ctx context.Context, sequence core.Sequence) (uint64, error) {
	number, err := n.Store.NextNumber(ctx, sequence)
	if err != nil {
		return 0, &numberError{cause: err}
	}
	return number, nil
}

// numberError retains both the Host failure classification and the store cause.
type numberError struct{ cause error }

// Error returns the underlying store failure text.
func (e *numberError) Error() string { return e.cause.Error() }

// Unwrap exposes both the Host classification and store cause.
func (e *numberError) Unwrap() []error {
	return []error{&host.Error{Kind: host.Failed, Message: e.cause.Error()}, e.cause}
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
	// Hosts resolves the node's current execution target.
	Hosts HostResolver[H]
	// Shells creates environments on resolved Hosts.
	Shells ShellEnvironmentFactory[H]
	// Environments owns the node's environments and their cleanup.
	Environments *Environments
	// Context identifies the node and its conversation root.
	Context NodeContext
	// Agent is the node's model-facing agent number.
	Agent uint64
	// Commands contains the commands offered by its shells.
	Commands *host.CommandSet
	// Feed reports command pages and their watches.
	Feed host.PageFeed
	// Numbers assigns durable command and shell numbers.
	Numbers host.Numbers
}

// Invoke runs one standard tool call. Refused input returns an error outcome;
// a Host failure returns its error, and cancellation returns ctx.Err().
// The context owns commands started by the call after Invoke returns.
func (s *ShellAccess[H]) Invoke(ctx context.Context, call session.ToolInvocation) (session.ToolOutcome, error) {
	outcome, err := runTool(ctx, call, s.Context.Node, s)
	if ctx.Err() != nil {
		return session.ToolOutcome{}, ctx.Err()
	}
	return outcome, err
}

// Write writes stdin to a running command of the current Host and returns
// its environment.
// A Host failure returns its error.
func (s *ShellAccess[H]) Write(
	ctx context.Context,
	command core.CommandID,
	stdin string,
) (host.ShellEnvironment, error) {
	_, environment, err := s.environment(ctx, nil, &command)
	if err != nil {
		return nil, err
	}
	if err := environment.Write(ctx, command, []byte(stdin)); err != nil {
		return nil, err
	}
	return environment, nil
}

// Abort stops a running command of the current Host and returns its environment.
// A Host failure returns its error.
func (s *ShellAccess[H]) Abort(ctx context.Context, command core.CommandID) (host.ShellEnvironment, error) {
	_, environment, err := s.environment(ctx, nil, &command)
	if err != nil {
		return nil, err
	}
	if err := environment.Abort(ctx, command); err != nil {
		return nil, err
	}
	return environment, nil
}

// environment resolves the current target and its node-owned shell environment.
func (s *ShellAccess[H]) environment(
	ctx context.Context,
	shell *core.ShellID,
	command *core.CommandID,
) (*environmentSlot, host.ShellEnvironment, error) {
	target, err := s.Hosts.Host(ctx, s.Context)
	if err != nil {
		return nil, nil, err
	}
	scope := EnvironmentScope{
		Root:     s.Context.Root,
		Node:     s.Context.Node,
		Agent:    s.Agent,
		Commands: s.Commands,
		Feed:     s.Feed,
		Numbers:  s.Numbers,
	}
	slot, environment, err := s.Environments.resolve(
		ctx,
		target.Key(),
		shell,
		command,
		func(ctx context.Context) (host.ShellEnvironment, error) { return s.Shells.Create(ctx, scope, target) },
	)
	if err != nil {
		return nil, nil, err
	}
	return slot, environment, nil
}

// shellOperations is the node access needed to dispatch a validated shell call.
type shellOperations interface {
	environment(context.Context, *core.ShellID, *core.CommandID) (*environmentSlot, host.ShellEnvironment, error)
	Write(context.Context, core.CommandID, string) (host.ShellEnvironment, error)
	Abort(context.Context, core.CommandID) (host.ShellEnvironment, error)
}

// runTool validates one tool's generated input and dispatches it to its Host.
func runTool(
	ctx context.Context,
	call session.ToolInvocation,
	node core.NodeID,
	s shellOperations,
) (session.ToolOutcome, error) {
	var environment host.ShellEnvironment
	var status host.CommandStatus
	var err error
	switch Standard(call.ToolName) {
	case Yield:
		input, decodeErr := decodeYieldInput(call.Input)
		if decodeErr != nil {
			return session.ErrorOutcome(inputRefusal(call.ToolName, decodeErr)), nil
		}
		return session.ToolOutcome{Effect: &session.ScheduleYield{DurationMS: uint32(input.DurationMS)}}, nil
	case ShellExec:
		return execTool(ctx, call, node, s)
	case ShellStatus, ShellAbort:
		input, decodeErr := decodeCommandInput(call.Input)
		if decodeErr != nil {
			return session.ErrorOutcome(inputRefusal(call.ToolName, decodeErr)), nil
		}
		command := core.CommandID(strconv.FormatUint(input.CommandID, 10))
		if Standard(call.ToolName) == ShellAbort {
			environment, err = s.Abort(ctx, command)
		} else {
			_, environment, err = s.environment(ctx, nil, &command)
		}
		if err == nil {
			status, err = environment.Status(command)
		}
	case ShellWrite:
		input, decodeErr := decodeShellWriteInput(call.Input)
		if decodeErr != nil {
			return session.ErrorOutcome(inputRefusal(call.ToolName, decodeErr)), nil
		}
		command := core.CommandID(strconv.FormatUint(input.CommandID, 10))
		environment, err = s.Write(ctx, command, string(input.Stdin))
		if err == nil {
			status, err = environment.Status(command)
		}
	default:
		return session.ErrorOutcome("Tool not found: " + call.ToolName), nil
	}
	if err != nil {
		return session.ToolOutcome{}, err
	}
	return commandOutcome(ctx, call, environment, status), nil
}

// commandOutcome reports a command's status as the call's result. Reporting
// an end releases the command's handle even if result fitting is cancelled.
func commandOutcome(
	ctx context.Context,
	call session.ToolInvocation,
	environment host.ShellEnvironment,
	status host.CommandStatus,
) session.ToolOutcome {
	if status.State.Phase != host.Running {
		defer environment.ReleaseCommand(context.WithoutCancel(ctx), status.CommandID)
	}
	return shellOutcome(ctx, status, call.Model.Model, call.RequestLimits)
}

// inputRefusal is the model-facing text refusing a tool's invalid input,
// naming the tool and the generated decoder's offending field.
func inputRefusal(tool string, err error) string {
	return tool + " input is invalid:\n" + err.Error()
}

func execTool(
	ctx context.Context, call session.ToolInvocation, node core.NodeID, s shellOperations,
) (session.ToolOutcome, error) {
	input, decodeErr := decodeShellExecInput(call.Input)
	if decodeErr != nil {
		return session.ErrorOutcome(inputRefusal(call.ToolName, decodeErr)), nil
	}
	var shell *core.ShellID
	target := host.ShellTarget{Kind: host.DefaultShell}
	if input.ShellID != nil {
		id := core.ShellID(strconv.FormatUint(*input.ShellID, 10))
		shell = &id
		target = host.ShellTarget{Kind: host.ExistingShell, ID: id}
	}
	slot, environment, err := s.environment(ctx, shell, nil)
	if err != nil {
		return session.ToolOutcome{}, err
	}
	if repeat, suppressed := slot.repeated(input.Script); suppressed {
		return repeat, nil
	}
	window, _ := host.NewObservationWindow(
		uint64(input.TimeoutMS),
	) // Generated validation guarantees the window's bounds.
	status, err := environment.Exec(
		ctx,
		host.ExecRequest{
			Script:    input.Script,
			Shell:     target,
			Window:    window,
			Caller:    host.JobCaller{Node: node, Generation: call.Generation},
			ToolUseID: call.ToolUseID,
		},
	)
	if err != nil {
		return session.ToolOutcome{}, err
	}
	return commandOutcome(ctx, call, environment, status), nil
}
