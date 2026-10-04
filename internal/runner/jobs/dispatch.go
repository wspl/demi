package jobs

import (
	"context"
	"errors"
	"fmt"
	"io"
	"strings"

	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/runner/commandpackages"
	"github.com/wspl/demi/internal/runner/process"
)

// Dispatcher runs declared commands for local clients and shell builtins alike.
// It validates argv and holds JSON output until it passes the leaf's schema.
// Fields are configured before use and remain unchanged while serving calls.
type Dispatcher struct {
	// Contexts resolves live execution authority.
	Contexts *Contexts
	// Services acquires native command services.
	Services *commandpackages.ServiceRegistry
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
	invocation commandsdk.InvocationContext[commandproto.LocalInvocation],
) (commandproto.Completion, error) {
	completion, err := d.invoke(ctx, invocation)
	return Reported(ctx, completion, err, invocation.Output)
}

var _ commandsdk.Handler[commandproto.LocalInvocation] = (*Dispatcher)(nil)

// Reported presents a local invocation's end: failures other than cancellation
// are written to stderr and become exit 1. The registration's management handler
// uses the same reporting as declared commands.
func Reported(
	ctx context.Context,
	completion commandproto.Completion,
	err error,
	output *commandsdk.Output,
) (commandproto.Completion, error) {
	if err == nil {
		return completion, nil
	}
	if errors.Is(err, context.Canceled) {
		return commandproto.Completion{}, err
	}
	if writeErr := output.Stderr(ctx, []byte(fmt.Sprintf("demi-runner: %v\n", err))); writeErr != nil {
		return commandproto.Completion{}, writeErr
	}
	return commandproto.Completion{ExitCode: 1}, nil
}

// invoke validates one local invocation before selecting its declared destination.
func (d *Dispatcher) invoke(
	ctx context.Context,
	invocation commandsdk.InvocationContext[commandproto.LocalInvocation],
) (commandproto.Completion, error) {
	if invocation.Request.Operation != process.Raw {
		return commandproto.Completion{}, fmt.Errorf("unknown operation: %s", invocation.Request.Operation)
	}
	raw, err := process.DecodeRawCommand(invocation.Request.Args)
	if err != nil {
		return commandproto.Completion{}, err
	}
	execution, err := d.Contexts.Lookup(raw.Context)
	if err != nil {
		return commandproto.Completion{}, err
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
		return commandproto.Completion{}, context.Canceled
	}
	return d.declaredCommand(ctx, invocation, execution, raw)
}

func (d *Dispatcher) declaredCommand(
	ctx context.Context,
	invocation commandsdk.InvocationContext[commandproto.LocalInvocation],
	execution *ExecutionContext,
	raw process.RawCommand,
) (commandproto.Completion, error) {
	root, ok := execution.Manifest.Roots[raw.Root]
	if !ok {
		return commandproto.Completion{}, fmt.Errorf("%s: not a root command of this manifest", raw.Root)
	}
	tree, err := commanddecl.DecodeManifestNode(root.Tree)
	if err != nil {
		return commandproto.Completion{}, err
	}
	selected, err := tree.Select(raw.Argv)
	if err != nil {
		return commandproto.Completion{}, err
	}
	parsed, err := selected.Parse(raw.Argv)
	if err != nil {
		return commandproto.Completion{}, err
	}
	if parsed.Help {
		return commandproto.Completion{}, invocation.Output.Stdout(
			ctx,
			[]byte(selected.Node.Help(strings.Join(selected.Path, " "))+"\n"),
		)
	}
	leaf, ok := selected.Node.(*commanddecl.Leaf[commanddecl.Binding])
	if !ok {
		return commandproto.Completion{}, errors.New("missing command leaf")
	}
	body, err := commandBody(ctx, invocation.Input, raw.Live, leaf.StdinField != nil)
	if err != nil {
		return commandproto.Completion{}, err
	}
	parsed, err = parsed.Validate(leaf, body)
	if err != nil {
		return commandproto.Completion{}, err
	}
	return d.invokeDeclared(ctx, invocation, execution, raw, parsed, leaf)
}

func commandBody(ctx context.Context, input *commandsdk.Input, live, hasBody bool) (*string, error) {
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
	invocation commandsdk.InvocationContext[commandproto.LocalInvocation],
	execution *ExecutionContext,
	binding commanddecl.Binding,
	parsed *commanddecl.Parsed,
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
	request := commandproto.Invocation{
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
	completion, err := (commandsdk.Exchange{Input: input, Output: response}).Run(ctx, invocation.Input, output)
	if err != nil {
		var exchange *commandsdk.ExchangeError
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

func readCommandBody(ctx context.Context, input *commandsdk.Input) ([]byte, error) {
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
	invocation commandsdk.InvocationContext[commandproto.LocalInvocation],
	execution *ExecutionContext,
	raw process.RawCommand,
	parsed *commanddecl.Parsed,
	leaf *commanddecl.Leaf[commanddecl.Binding],
) (commandproto.Completion, error) {
	var err error
	output := &commandOutput{output: invocation.Output}
	if parsed.JSON {
		output.schema = leaf.JSONOutput()
	}
	clearHint, err := runningHint(ctx, execution.Connection, execution.JobID, leaf.RunningHint)
	if err != nil {
		return commandproto.Completion{}, err
	}
	defer clearHint()
	var code uint8
	if binding, native := leaf.Binding(); native {
		code, err = d.nativeCommand(ctx, invocation, execution, binding, parsed, output)
		if err != nil {
			return commandproto.Completion{}, err
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
			return commandproto.Completion{}, err
		}
	}
	if err := output.finish(ctx, code); err != nil {
		return commandproto.Completion{}, err
	}
	return commandproto.Completion{ExitCode: code}, nil
}
