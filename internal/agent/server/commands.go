package server

import (
	"context"
	"errors"
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
)

// agentLeaf binds one generated command contract to the invoking live tree.
func agentLeaf[H host.Host, A any](
	t *Tree[H],
	name, summary string,
	shape commandContract[A],
	positionals []string,
	stdin *string,
	run func(context.Context, *Tree[H], core.NodeID, bool, A, host.RPCPort) (uint8, error),
) (host.Declared, error) {
	var input *declare.Schema
	if shape.input != nil {
		var err error
		input, err = declare.NewSchema(shape.input)
		if err != nil {
			return host.Declared{}, err
		}
	}
	leaf := declare.Leaf[declare.NativeOperation]{
		Name:       name,
		Summary:    summary,
		Input:      input,
		StdinField: stdin,
		Kind:       &declare.RPC[declare.NativeOperation]{},
	}
	if positionals != nil {
		leaf.Positionals = &positionals
	}
	if shape.output != nil {
		result, err := declare.NewSchema(shape.output)
		if err != nil {
			return host.Declared{}, err
		}
		leaf.Output = &declare.LeafOutput{JSON: result}
	}
	handler := host.TypedRPC(
		shape.decode,
		func(ctx context.Context, call host.Call[A], port host.RPCPort) (uint8, error) {
			if call.Invocation.Caller == nil {
				return 0, &host.RPCError{Kind: host.HandlerFailed, Message: "the command runs only in an agent's job"}
			}
			root, err := core.ParseNodeID(call.Invocation.Context.Conversation)
			if err != nil {
				return 0, err
			}
			tree := t.server.Tree(root)
			if tree == nil {
				return 0, &host.RPCError{
					Kind:    host.HandlerFailed,
					Message: fmt.Sprintf("the conversation %s is not open", root),
				}
			}
			return run(ctx, tree, call.Invocation.Caller.Node, call.Invocation.JSON, call.Args, port)
		},
	)
	if name == "spawn" {
		leaf.SuccessOutput = new(`stdout is "subagentId: <id>"; creation succeeded, not necessarily execution`)
		leaf.FailureOutput = new("non-zero exit with the creation failure reason on stderr")
	}
	return host.Leaf(leaf, handler), nil
}

func (t *Tree[H]) commands(inherited *host.CommandSet, spawning bool) (*host.CommandSet, error) {
	commands := inherited.Filter(func([]string) bool { return true })
	children := []host.Declared{}
	if spawning {
		leaf, err := agentLeaf(t, "spawn", spawnSummary, spawnContract(), nil, new("prompt"), spawnCommand[H])
		if err != nil {
			return nil, err
		}
		available := t.profileNames()
		children = append(
			children,
			leaf.Describe("prompt", spawnPrompt).
				Describe("profile", fmt.Sprintf("Named subagent profile; omit to inherit the parent's model, prompt, "+
					"Host and commands. "+
					"Available: %s.", available)),
		)
	}
	send, err := agentLeaf(t, "send", sendSummary, sendContract(), []string{"id"}, new("message"), sendCommand[H])
	if err != nil {
		return nil, err
	}
	children = append(children, send)
	if spawning {
		abort, err := agentLeaf(t, "abort", abortSummary, abortContract(), []string{"id"}, nil, abortCommand[H])
		if err != nil {
			return nil, err
		}
		resume, err := agentLeaf(
			t,
			"resume",
			resumeSummary,
			resumeContract(),
			[]string{"id"},
			new("message"),
			resumeCommand[H],
		)
		if err != nil {
			return nil, err
		}
		children = append(children, abort, resume)
	}
	list, err := agentLeaf(t, "list", listSummary, listContract(), nil, nil, listCommand[H])
	if err != nil {
		return nil, err
	}
	show, err := agentLeaf(t, "show", showSummary, showContract(), []string{"id"}, nil, showCommand[H])
	if err != nil {
		return nil, err
	}
	children = append(children, list, show)
	summary := "Agent tree: spawn and manage your own children; send, list and show any live agent."
	if !spawning {
		summary = "Agent tree communication: this session may not spawn subagents; send, list and show any live agent."
	}
	agent := host.Group("agent", summary, children...)
	shell, err := t.shellGroup()
	if err != nil {
		return nil, err
	}
	return graftCommands(commands, agent, shell)
}

func spawnCommand[H host.Host](
	ctx context.Context,
	t *Tree[H],
	caller core.NodeID,
	jsonOutput bool,
	args spawnArgs,
	port host.RPCPort,
) (uint8, error) {
	prompt := core.Trim(args.Prompt)
	if prompt == "" {
		return commandFail(ctx, port, "spawn", errors.New("prompt must not be empty"))
	}
	input := &spawnInput{Prompt: prompt, ProfileName: args.Profile}
	if args.Description != nil {
		input.Description = *args.Description
	}
	if args.NoSubagents != nil {
		input.IsSpawnForbidden = *args.NoSubagents
	}
	var request string
	if args.RequestID != nil {
		request = *args.RequestID
	} else {
		request = t.server.deps.IDs.NextID()
	}
	child, err := t.start(ctx, caller, input, request, port)
	if err != nil {
		return commandFail(ctx, port, "spawn", err)
	}
	return startedOutput(ctx, port, jsonOutput, child)
}

func resumeCommand[H host.Host](
	ctx context.Context,
	t *Tree[H],
	caller core.NodeID,
	jsonOutput bool,
	args resumeArgs,
	port host.RPCPort,
) (uint8, error) {
	message := core.Trim(args.Message)
	if message == "" {
		return commandFail(ctx, port, "resume", errors.New("message must not be empty"))
	}
	records, err := t.store.Children(ctx, caller)
	if err != nil {
		return commandFail(ctx, port, "resume", err)
	}
	var id core.NodeID
	for _, record := range records {
		if record.Number == args.ID {
			id = record.ID
			break
		}
	}
	if id == "" {
		return commandFail(ctx, port, "resume", fmt.Errorf("no archived subagent %d (see `demi agent list`)", args.ID))
	}
	var request string
	if args.RequestID != nil {
		request = *args.RequestID
	} else {
		request = t.server.deps.IDs.NextID()
	}
	child, err := t.start(ctx, caller, &resumeInput{ID: id, Message: message}, request, port)
	if err != nil {
		return commandFail(ctx, port, "resume", err)
	}
	return startedOutput(ctx, port, jsonOutput, child)
}

func sendCommand[H host.Host](
	ctx context.Context,
	t *Tree[H],
	caller core.NodeID,
	jsonOutput bool,
	args sendArgs,
	port host.RPCPort,
) (uint8, error) {
	if core.IsBlank(args.Message) {
		return commandFail(ctx, port, "send", errors.New("message must not be empty"))
	}
	target, err := t.sendMessage(ctx, caller, args.ID, core.Trim(args.Message))
	if err != nil {
		return commandFail(ctx, port, "send", err)
	}
	if jsonOutput {
		return commandJSON(ctx, port, sent{ID: target, Accepted: true})
	}
	return commandOut(ctx, port, fmt.Sprintf("sent to %d\n", target))
}

func abortCommand[H host.Host](
	ctx context.Context,
	t *Tree[H],
	caller core.NodeID,
	jsonOutput bool,
	args abortArgs,
	port host.RPCPort,
) (uint8, error) {
	var id core.NodeID
	for _, child := range t.childrenOf(caller) {
		if child.node.record.Number == args.ID {
			id = child.node.ID()
			break
		}
	}
	if id == "" {
		return commandFail(ctx, port, "abort", fmt.Errorf("%d is not one of your running children", args.ID))
	}
	if err := t.abortChild(ctx, id); err != nil {
		return commandFail(ctx, port, "abort", err)
	}
	if jsonOutput {
		return commandJSON(ctx, port, aborted{ID: args.ID, Aborted: true})
	}
	return commandOut(ctx, port, fmt.Sprintf("aborted %d\n", args.ID))
}

func startedOutput(ctx context.Context, port host.RPCPort, jsonOutput bool, child uint64) (uint8, error) {
	if jsonOutput {
		return commandJSON(ctx, port, started{SubagentID: child})
	}
	return commandOut(ctx, port, fmt.Sprintf("subagentId: %d\n", child))
}

// commandJSON writes a declared command result with serde-compatible JSON bytes.
func commandJSON(ctx context.Context, port host.RPCPort, value any) (uint8, error) {
	data, err := contract.EncodeJSON(value)
	if err != nil {
		return 0, err
	}
	return commandOut(ctx, port, string(data)+"\n")
}

func commandOut(ctx context.Context, port host.RPCPort, text string) (uint8, error) {
	return 0, port.Stdout(ctx, []byte(text))
}

func commandFail(ctx context.Context, port host.RPCPort, verb string, err error) (uint8, error) {
	return 1, port.Stderr(ctx, []byte(fmt.Sprintf("demi agent %s: %s\n", verb, err)))
}

func graftCommands(commands *host.CommandSet, agent, shell host.Declared) (*host.CommandSet, error) {
	hasDemi := false
	for _, root := range commands.Declarations() {
		if declare.Name(root) == "demi" {
			hasDemi = true
		}
	}
	if hasDemi {
		if err := commands.Graft([]string{"demi"}, agent); err != nil {
			return nil, err
		}
		if err := commands.Graft([]string{"demi"}, shell); err != nil {
			return nil, err
		}
	} else if err := commands.Register(host.Group("demi", "Demi agent runtime commands.", agent, shell)); err != nil {
		return nil, err
	}
	return commands, nil
}

func (t *Tree[H]) profileNames() string {
	names := []string{}
	for _, profile := range t.profiles {
		names = append(names, profile.Name)
	}
	available := strings.Join(names, ", ")
	if available == "" {
		available = "none"
	}
	return available
}
