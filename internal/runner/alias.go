package runner

import (
	"context"
	"errors"
	"os"
	"strings"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/process"
)

// commandAlias forwards a declared root's arguments untouched to its live runner.
func commandAlias(ctx context.Context, root string, argv []string) (uint8, error) {
	env := processEnvironment()
	endpoint, ok := env[process.EndpointEnv]
	if !ok {
		return 0, errors.New("command requires a runner execution context")
	}
	execution, ok := env[process.ContextEnv]
	if !ok {
		return 0, errors.New("missing command context")
	}
	stdin, err := process.StandardFile(ctx, 0)
	if err != nil {
		return 0, err
	}
	defer func() { _ = stdin.Close() }() // Forward owns IO once called; this also covers setup failures.
	live, err := process.IsLive(stdin, env)
	if err != nil {
		return 0, err
	}
	request, err := process.NewRawCommand(execution, root, argv, live)
	if err != nil {
		return 0, err
	}
	args, err := request.MarshalJSON()
	if err != nil {
		return 0, err
	}
	invocation, err := aliasInvocation(args, env)
	if err != nil {
		return 0, err
	}
	stdout, err := process.StandardFile(ctx, 1)
	if err != nil {
		return 0, err
	}
	defer func() { _ = stdout.Close() }() // Duplicated descriptor, also closed by Forward.
	stderr, err := process.StandardFile(ctx, 2)
	if err != nil {
		return 0, err
	}
	defer func() { _ = stderr.Close() }() // Duplicated descriptor, also closed by Forward.
	completion, err := process.Forward(
		ctx,
		endpoint,
		invocation,
		process.Stdio{Stdin: stdin, Stdout: stdout, Stderr: stderr},
	)
	if ctx.Err() != nil {
		return 130, nil
	}
	return completion.ExitCode, err
}

// processEnvironment snapshots the device environment for jobs and command aliases.
func processEnvironment() map[string]string {
	values := make(map[string]string)
	for _, entry := range os.Environ() {
		name, value, ok := strings.Cut(entry, "=")
		if ok {
			values[name] = value
		}
	}
	return values
}

func aliasInvocation(args []byte, env map[string]string) (commandwire.LocalInvocation, error) {
	cwd, err := os.Getwd()
	if err != nil {
		return commandwire.LocalInvocation{}, err
	}
	id, err := uuid.NewRandom()
	if err != nil {
		return commandwire.LocalInvocation{}, err
	}
	return commandwire.LocalInvocation{
		Operation:    process.Raw,
		InvocationID: strings.ReplaceAll(id.String(), "-", ""),
		Args:         args,
		Cwd:          cwd,
		Env:          env,
	}, nil
}
