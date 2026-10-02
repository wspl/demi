package jobs

import (
	"context"
	"errors"
	"fmt"
	"io"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/process"
)

// Dispatcher runs declared commands for local clients and shell builtins alike.
// It validates argv and holds JSON output until it passes the leaf's schema.
// Fields are configured before use and remain unchanged while serving calls.
type Dispatcher struct {
	Contexts *Contexts
	Services *cmdpkgs.ServiceHandle
	Pipes    *process.PipeClient
}

// Operations returns the raw command operation served by the dispatcher.
func (d *Dispatcher) Operations() []string {
	return []string{process.Raw}
}

// Invoke authenticates the execution context, parses the declaration and routes
// native work to a service or an RPC call to the backend. Cancellation joins IO.
func (d *Dispatcher) Invoke(ctx context.Context, invocation cmdsdk.InvocationContext[commandwire.LocalInvocation]) (commandwire.Completion, error) {
	completion, err := d.command(ctx, invocation)
	return Reported(ctx, completion, err, invocation.Output)
}

var _ cmdsdk.Handler[commandwire.LocalInvocation] = (*Dispatcher)(nil)

// Reported presents a local invocation's end: failures other than cancellation
// are written to stderr and become exit 1. The registration's management handler
// uses the same reporting as declared commands.
func Reported(ctx context.Context, completion commandwire.Completion, err error, output *cmdsdk.Output) (commandwire.Completion, error) {
	if err == nil {
		return completion, nil
	}
	if errors.Is(err, context.Canceled) {
		return commandwire.Completion{}, err
	}
	if writeErr := output.Stderr(ctx, []byte(fmt.Sprintf("demi-runner: %v\n", err))); writeErr != nil {
		return commandwire.Completion{}, writeErr
	}
	return commandwire.Completion{ExitCode: 1}, nil
}

// command validates one local invocation before selecting its declared destination.
func (d *Dispatcher) command(ctx context.Context, inv cmdsdk.InvocationContext[commandwire.LocalInvocation]) (commandwire.Completion, error) {
	if inv.Request.Operation != process.Raw {
		return commandwire.Completion{}, fmt.Errorf("unknown operation: %s", inv.Request.Operation)
	}
	raw, err := process.DecodeRawCommand(inv.Request.Args)
	if err != nil {
		return commandwire.Completion{}, err
	}
	execution, err := d.Contexts.Lookup(raw.Context)
	if err != nil {
		return commandwire.Completion{}, err
	}
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	watched := make(chan struct{})
	go func() {
		defer close(watched)
		select {
		case <-execution.Done():
			cancel()
		case <-ctx.Done():
		}
	}()
	defer func() { cancel(); <-watched }()
	if execution.lifetime.Err() != nil {
		return commandwire.Completion{}, context.Canceled
	}
	root, ok := execution.Manifest.Roots[raw.Root]
	if !ok {
		return commandwire.Completion{}, fmt.Errorf("%s: not a root command of this manifest", raw.Root)
	}
	tree, err := declare.DecodeManifestNode(root.Tree)
	if err != nil {
		return commandwire.Completion{}, err
	}
	selected, err := tree.Select(raw.Argv)
	if err != nil {
		return commandwire.Completion{}, err
	}
	parsed, err := selected.Parse(raw.Argv)
	if err != nil {
		return commandwire.Completion{}, err
	}
	if parsed.Help {
		return commandwire.Completion{}, inv.Output.Stdout(ctx, []byte(selected.Node.Help(strings.Join(selected.Path, " "))+"\n"))
	}
	leaf := declare.AsLeaf(selected.Node)
	if leaf == nil {
		return commandwire.Completion{}, errors.New("missing command leaf")
	}
	var body *string
	if leaf.StdinField != nil {
		bytes := []byte{}
		if !raw.Live {
			for {
				chunk, err := inv.Input.Next(ctx)
				if errors.Is(err, io.EOF) {
					break
				}
				if err != nil {
					return commandwire.Completion{}, err
				}
				if len(bytes)+len(chunk) > 1024*1024 {
					return commandwire.Completion{}, errors.New("command body exceeds 1 MiB")
				}
				bytes = append(bytes, chunk...)
			}
		}
		if !utf8.Valid(bytes) {
			return commandwire.Completion{}, errors.New("invalid utf-8 sequence")
		}
		text := string(bytes)
		body = &text
	}
	parsed, err = parsed.Validate(leaf, body)
	if err != nil {
		return commandwire.Completion{}, err
	}
	output := &commandOutput{output: inv.Output}
	if parsed.JSON {
		output.schema = leaf.JSONOutput()
	}
	clearHint, err := runningHint(ctx, execution.Connection, execution.JobID, leaf.RunningHint)
	if err != nil {
		return commandwire.Completion{}, err
	}
	defer clearHint()
	var code uint8
	if binding := leaf.Binding(); binding != nil {
		descriptor, ok := execution.Manifest.Packages[binding.DescriptorHash]
		if !ok {
			return commandwire.Completion{}, errors.New("native descriptor is not in this manifest")
		}
		resolver := &jobArtifacts{contexts: d.Contexts}
		registration := d.Services.Invoking(inv.Request.InvocationID, descriptor.ID, resolver)
		defer registration.Release()
		resident, err := d.Services.Acquire(ctx, descriptor, resolver, execution.Connection)
		if err != nil {
			return commandwire.Completion{}, err
		}
		args, err := parsed.Values.MarshalJSON()
		if err != nil {
			return commandwire.Completion{}, err
		}
		request := commandwire.Invocation{Operation: binding.Operation, InvocationID: inv.Request.InvocationID, Context: execution.Command, Args: args, Cwd: inv.Request.Cwd, Env: inv.Request.Env, Edits: &execution.Edits, JSON: &parsed.JSON}
		input, response, err := resident.Client().Invoke(ctx, request)
		if err != nil {
			if errors.Is(err, context.Canceled) {
				return commandwire.Completion{}, err
			}
			return commandwire.Completion{}, resident.Failure(ctx, err)
		}
		completion, err := (cmdsdk.Exchange{Input: input, Output: response}).Run(ctx, inv.Input, output)
		if err != nil {
			var exchange *cmdsdk.ExchangeError
			if errors.As(err, &exchange) && exchange.Side == "service" && !errors.Is(err, context.Canceled) {
				err = resident.Failure(ctx, err)
			}
			return commandwire.Completion{}, err
		}
		if completion.Error != nil {
			if err := output.Stderr(ctx, []byte(completion.Error.Code+": "+completion.Error.Message+"\n")); err != nil {
				return commandwire.Completion{}, err
			}
		}
		code = completion.ExitCode
	} else {
		code, err = invokeRPC(ctx, d.Pipes, execution, raw, parsed, inv, output, !raw.Live && leaf.StdinField == nil)
		if err != nil {
			return commandwire.Completion{}, err
		}
	}
	if err := output.finish(ctx, code); err != nil {
		return commandwire.Completion{}, err
	}
	return commandwire.Completion{ExitCode: code}, nil
}
