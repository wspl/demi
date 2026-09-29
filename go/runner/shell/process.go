package shell

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"strconv"

	"github.com/wspl/demi/go/runner/process"
	"mvdan.cc/sh/v3/interp"
)

// processCall routes the process-sensitive builtins through ExecHandlers.
// mvdan's exec already starts a child and exits only the interpreter; it does
// not replace this process. command/builtin must not bypass the safeguards.
func processCall(ctx context.Context, args []string) ([]string, error) {
	if interp.HandlerCtx(ctx).IsFunction(args[0]) {
		return args, nil
	}
	index := 0
	for index < len(args)-1 && (args[index] == "command" || args[index] == "builtin") {
		index++
	}
	switch args[index] {
	case "exec":
		return execArguments(args[index+1:]), nil
	case "kill", "suspend", "fg", "umask", "ulimit":
		return append([]string{"__demi_process_builtin"}, args[index:]...), nil
	}
	return args, nil
}

type executionContextKey struct{}

func (s *scope) processes(_ interp.ExecHandlerFunc) interp.ExecHandlerFunc {
	return func(ctx context.Context, args []string) error {
		ctx = context.WithValue(ctx, executionContextKey{}, s.options.Env["DEMI_CONTEXT_ID"])
		if args[0] == "__demi_usage" && len(args) == 2 {
			return builtinUsage(ctx, args[1])
		}
		if args[0] == "__demi_process_builtin" {
			if len(args) == 1 {
				return diagnostic(ctx, args[0], errors.New("missing process builtin"))
			}
			return processBuiltin(ctx, args[1:])
		}
		// exec calls the exec handler even when its operand is a builtin.
		switch args[0] {
		case "kill", "suspend", "fg", "umask", "ulimit":
			return processBuiltin(ctx, args)
		}
		if interp.IsBuiltin(args[0]) {
			return interp.HandlerCtx(ctx).Builtin(ctx, args)
		}
		return runProcess(ctx, args)
	}
}

func processBuiltin(ctx context.Context, args []string) error {
	switch args[0] {
	case "umask":
		return umask(ctx, args)
	case "ulimit":
		return ulimit(ctx, args)
	case "suspend":
		return diagnostic(ctx, "suspend", errors.New("a job cannot suspend the runner it runs in"))
	case "fg":
		return diagnostic(ctx, "fg", errors.New("no job control"))
	case "kill":
		skip := false
		for _, arg := range args[1:] {
			if skip {
				skip = false
				continue
			}
			if arg == "-l" || arg == "-L" {
				break
			}
			if arg == "-s" || arg == "-n" {
				skip = true
				continue
			}
			pid, err := strconv.Atoi(arg)
			if err == nil && (pid == 0 || pid == os.Getpid()) {
				return diagnostic(ctx, "kill", errors.New("a job cannot signal the runner it runs in"))
			}
		}
		return kill(ctx, args)
	}
	return runProcess(ctx, args)
}

// runProcess starts PATH programs with the job's streams and environment.
// Platform containment kills the child's process group on cancellation.
func runProcess(ctx context.Context, args []string) error {
	hc := interp.HandlerCtx(ctx)
	path, err := interp.LookPathDir(hc.Dir, hc.Env, args[0])
	if err != nil {
		if hc.Exec {
			err = fmt.Errorf("exec: %s: not found", args[0])
		}
		if _, failure := fmt.Fprintln(hc.Stderr, err); failure != nil {
			return failure
		}
		return interp.ExitStatus(127)
	}
	setup, err := childAttributes(ctx)
	if err != nil {
		return err
	}
	if setup.Mask != nil || len(setup.Limits) != 0 {
		setup.Path, setup.Args = path, append([]string(nil), args...)
		if hc.Argv0 != nil {
			setup.Args[0] = *hc.Argv0
		}
		path, args, err = helperArgs(setup)
		if err != nil {
			return err
		}
	}
	return launch(ctx, path, args, hc.Stdout, hc.Stderr)
}

// launch starts with the runner's common retry policy, then reaps exactly once.
func launch(ctx context.Context, path string, args []string, stdout, stderr io.Writer) error {
	child, err := process.Start(ctx, func() (*startedProcess, error) {
		return startProcess(ctx, path, args, stdout, stderr)
	})
	if err != nil {
		if ctx.Err() != nil {
			return ctx.Err()
		}
		return diagnostic(ctx, args[0], err)
	}
	err = child.cmd.Wait()
	child.cleanup()
	outputErr := child.output.finish(ctx)
	if ctx.Err() != nil {
		return ctx.Err()
	}
	var exited *exec.ExitError
	if errors.As(err, &exited) {
		return interp.ExitStatus(processStatus(exited))
	}
	if err != nil {
		return err
	}
	return outputErr
}

type startedProcess struct {
	cmd     *exec.Cmd
	output  *processOutput
	cleanup func()
}

func startProcess(ctx context.Context, path string, args []string, stdout, stderr io.Writer) (_ *startedProcess, result error) {
	hc := interp.HandlerCtx(ctx)
	cmd := exec.CommandContext(ctx, path, args[1:]...)
	cmd.Args = append([]string(nil), args...)
	if hc.Argv0 != nil && (len(args) < 2 || args[1] != helperFlag) {
		cmd.Args[0] = *hc.Argv0
	}
	cmd.Dir = hc.Dir
	env := environment(hc.Env)
	if id, ok := ctx.Value(executionContextKey{}).(string); ok && id != "" {
		env["DEMI_CONTEXT_ID"] = id
	}
	for name, value := range env {
		cmd.Env = append(cmd.Env, name+"="+value)
	}
	if cmd.Env == nil {
		cmd.Env = []string{}
	}
	if hc.ClearEnv {
		cmd.Env = []string{}
	}
	cmd.Stdin = hc.Stdin
	output := &processOutput{}
	defer func() {
		if result != nil {
			// No child owns these pipes after a failed start; closing them joins copies.
			_ = output.finish(ctx)
		}
	}()
	var err error
	cmd.Stdout, err = output.connect(stdout)
	if err != nil {
		return nil, err
	}
	cmd.Stderr, err = output.connect(stderr)
	if err != nil {
		return nil, err
	}
	closeFiles, err := hc.CommandFiles(ctx, cmd)
	if err != nil {
		return nil, err
	}

	cleanup, err := contain(cmd)
	if err != nil {
		closeFiles()
		return nil, err
	}
	err = cmd.Start()
	if err != nil {
		cleanup()
		closeFiles()
		return nil, err
	}
	output.closeWriters()
	return &startedProcess{cmd: cmd, output: output, cleanup: func() {
		cleanup()
		closeFiles()
	}}, nil
}
