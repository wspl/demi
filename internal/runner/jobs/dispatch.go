package jobs

import (
	"context"
	"errors"
	"fmt"
	"io"
	"strings"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/process"
)

// Dispatcher runs declared commands for local clients and shell builtins alike.
// It validates argv and holds JSON output until it passes the leaf's schema.
// Fields are configured before use and remain unchanged while serving calls.
type Dispatcher struct {
	// Contexts resolves live execution authority.
	Contexts *Contexts
	// Services acquires native command services.
	Services *cmdpkgs.ServiceRegistry
	// Pipes transfers callback input and output.
	Pipes *process.PipeClient
}

// Operations returns the raw command operation served by the dispatcher.
func (d *Dispatcher) Operations() []string {
	return []string{process.Raw}
}

// Invoke authenticates the execution context, parses the declaration and routes
// native work to a service or an RPC call to the backend. Cancellation joins IO.
func (d *Dispatcher) Invoke(
	ctx context.Context,
	invocation cmdsdk.InvocationContext[commandwire.LocalInvocation],
) (commandwire.Completion, error) {
	completion, err := d.invoke(ctx, invocation)
	return Reported(ctx, completion, err, invocation.Output)
}

var _ cmdsdk.Handler[commandwire.LocalInvocation] = (*Dispatcher)(nil)

// Reported presents a local invocation's end: failures other than cancellation
// are written to stderr and become exit 1. The registration's management handler
// uses the same reporting as declared commands.
func Reported(
	ctx context.Context,
	completion commandwire.Completion,
	err error,
	output *cmdsdk.Output,
) (commandwire.Completion, error) {
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

// invoke validates one local invocation before selecting its declared destination.
func (d *Dispatcher) invoke(
	ctx context.Context,
	invocation cmdsdk.InvocationContext[commandwire.LocalInvocation],
) (commandwire.Completion, error) {
	if invocation.Request.Operation != process.Raw {
		return commandwire.Completion{}, fmt.Errorf("unknown operation: %s", invocation.Request.Operation)
	}
	raw, err := process.DecodeRawCommand(invocation.Request.Args)
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
	defer func() {
		cancel()
		<-watched
	}()
	if execution.lifetime.Err() != nil {
		return commandwire.Completion{}, context.Canceled
	}
	return d.declaredCommand(ctx, invocation, execution, raw)
}

func (d *Dispatcher) declaredCommand(
	ctx context.Context,
	invocation cmdsdk.InvocationContext[commandwire.LocalInvocation],
	execution *ExecutionContext,
	raw process.RawCommand,
) (commandwire.Completion, error) {
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
		return commandwire.Completion{}, invocation.Output.Stdout(
			ctx,
			[]byte(selected.Node.Help(strings.Join(selected.Path, " "))+"\n"),
		)
	}
	leaf, ok := selected.Node.(*declare.Leaf[declare.Binding])
	if !ok {
		return commandwire.Completion{}, errors.New("missing command leaf")
	}
	body, err := commandBody(ctx, invocation.Input, raw.Live, leaf.StdinField != nil)
	if err != nil {
		return commandwire.Completion{}, err
	}
	parsed, err = parsed.Validate(leaf, body)
	if err != nil {
		return commandwire.Completion{}, err
	}
	return d.invokeDeclared(ctx, invocation, execution, raw, parsed, leaf)
}

func commandBody(ctx context.Context, input *cmdsdk.Input, live, hasBody bool) (*string, error) {
	if !hasBody {
		return nil, nil
	}
	bytes := []byte{}
	if !live {
		var err error
		bytes, err = readCommandBody(ctx, input)
		if err != nil {
			return nil, err
		}
	}
	if err := contract.CheckUTF8(bytes); err != nil {
		return nil, err
	}
	text := string(bytes)
	return &text, nil
}

func (d *Dispatcher) nativeCommand(
	ctx context.Context,
	invocation cmdsdk.InvocationContext[commandwire.LocalInvocation],
	execution *ExecutionContext,
	binding declare.Binding,
	parsed *declare.Parsed,
	output *commandOutput,
) (uint8, error) {
	descriptor, ok := execution.Manifest.Packages[binding.DescriptorHash]
	if !ok {
		return 0, errors.New("native descriptor is not in this manifest")
	}
	resolver := &jobArtifacts{contexts: d.Contexts}
	registration := d.Services.Invoking(invocation.Request.InvocationID, descriptor.ID, resolver)
	defer registration.Release()
	resident, err := d.Services.Acquire(ctx, descriptor, resolver, execution.Connection)
	if err != nil {
		return 0, err
	}
	args, err := parsed.Values.MarshalJSON()
	if err != nil {
		return 0, err
	}
	request := commandwire.Invocation{
		Operation:    binding.Operation,
		InvocationID: invocation.Request.InvocationID,
		Context:      execution.Command,
		Args:         args,
		Cwd:          invocation.Request.Cwd,
		Env:          invocation.Request.Env,
		Edits:        &execution.Edits,
		JSON:         &parsed.JSON,
	}
	input, response, err := resident.Client().Invoke(ctx, request)
	if err != nil {
		if errors.Is(err, context.Canceled) {
			return 0, err
		}
		return 0, resident.Failure(ctx, err)
	}
	completion, err := (cmdsdk.Exchange{Input: input, Output: response}).Run(ctx, invocation.Input, output)
	if err != nil {
		var exchange *cmdsdk.ExchangeError
		if errors.As(err, &exchange) && exchange.Side == "service" && !errors.Is(err, context.Canceled) {
			err = resident.Failure(ctx, err)
		}
		return 0, err
	}
	if completion.Error != nil {
		if err := output.Stderr(ctx, []byte(completion.Error.Code+": "+completion.Error.Message+"\n")); err != nil {
			return 0, err
		}
	}
	return completion.ExitCode, nil
}

func readCommandBody(ctx context.Context, input *cmdsdk.Input) ([]byte, error) {
	bytes := []byte{}
	for {
		chunk, err := input.Next(ctx)
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return nil, err
		}
		if len(bytes)+len(chunk) > 1024*1024 {
			return nil, errors.New("command body exceeds 1 MiB")
		}
		bytes = append(bytes, chunk...)
	}
	return bytes, nil
}

func (d *Dispatcher) invokeDeclared(
	ctx context.Context,
	invocation cmdsdk.InvocationContext[commandwire.LocalInvocation],
	execution *ExecutionContext,
	raw process.RawCommand,
	parsed *declare.Parsed,
	leaf *declare.Leaf[declare.Binding],
) (commandwire.Completion, error) {
	var err error
	output := &commandOutput{output: invocation.Output}
	if parsed.JSON {
		output.schema = leaf.JSONOutput()
	}
	clearHint, err := runningHint(ctx, execution.Connection, execution.JobID, leaf.RunningHint)
	if err != nil {
		return commandwire.Completion{}, err
	}
	defer clearHint()
	var code uint8
	if binding, native := leaf.Binding(); native {
		code, err = d.nativeCommand(ctx, invocation, execution, binding, parsed, output)
		if err != nil {
			return commandwire.Completion{}, err
		}
	} else {
		code, err = invokeRPC(
			ctx,
			d.Pipes,
			execution,
			raw,
			parsed,
			invocation,
			output,
			!raw.Live && leaf.StdinField == nil,
		)
		if err != nil {
			return commandwire.Completion{}, err
		}
	}
	if err := output.finish(ctx, code); err != nil {
		return commandwire.Completion{}, err
	}
	return commandwire.Completion{ExitCode: code}, nil
}
