package engine

import (
	"context"
	"errors"
	"fmt"
	"os/exec"
	"syscall"

	"github.com/wspl/demi/internal/runner/process"
	"mvdan.cc/sh/v3/expand"
	"mvdan.cc/sh/v3/interp"
)

// external executes system utilities under the job's process ownership.
func (e *execution) external(ctx context.Context, args []string) error {
	handler := interp.HandlerCtx(ctx)
	state := handler.Scope().(*interpreterScope)
	path, err := interp.LookPathDir(handler.Dir, handler.Env, shellPath(args[0]))
	if err != nil {
		_, _ = fmt.Fprintln(handler.Stderr, err)
		return interp.ExitStatus(127)
	}
	cmd := externalCommand(ctx, path, args)
	closeStreams, err := borrowStreams(ctx, cmd)
	if err != nil {
		return err
	}
	defer closeStreams()
	launchDone := make(chan struct{})
	finishedStart := false
	finishDescriptors, err := extraDescriptors(ctx, launchDone, cmd, handler.Descriptors())
	if err != nil {
		return err
	}
	defer func() {
		if !finishedStart {
			close(launchDone)
		}
		_ = finishDescriptors()
	}() // Start failures have no output to forward.
	child := process.Wrap(cmd, true, state.attributes)
	releaseLaunch, err := e.beginLaunch(ctx)
	if err != nil {
		return err
	}
	err = child.Start(ctx)
	close(launchDone)
	finishedStart = true
	releaseLaunch()
	if err != nil {
		return externalStartError(ctx, handler, cmd, path, err)
	}
	release := state.register(child)
	defer release()
	state.Waiting(1)
	defer state.Waiting(-1)
	status := child.Wait(ctx)
	if err := finishDescriptors(); err != nil {
		return err
	}
	if ctx.Err() != nil {
		return ctx.Err()
	}
	return externalStatus(status)
}

// exported supplies only variables marked for child inheritance.
func exported(env expand.Environ) []string {
	result := make([]string, 0)
	env.Each(func(name string, value expand.Variable) bool {
		if value.Exported && value.IsSet() {
			result = append(result, name+"="+value.String())
		}
		return true
	})
	return result
}

func externalCommand(ctx context.Context, path string, args []string) *exec.Cmd {
	handler := interp.HandlerCtx(ctx)
	cmd := exec.Command(path, args[1:]...)
	cmd.Dir = handler.Dir
	cmd.Env = exported(handler.Env)
	if options, ok := ctx.Value(execKey{}).(execOptions); ok {
		if options.empty {
			cmd.Env = []string{}
		}
		if options.argv0 != "" {
			cmd.Args[0] = options.argv0
		}
	}
	cmd.Stdin = handler.Stdin
	cmd.Stdout = handler.Stdout
	cmd.Stderr = handler.Stderr
	return cmd
}

func externalStatus(status process.Exit) error {
	if status.Error != nil {
		return errors.New(*status.Error)
	}
	if status.Code != nil {
		if *status.Code == 0 {
			return nil
		}
		return interp.ExitStatus(uint8(*status.Code))
	}
	if status.Signal != nil {
		return interp.ExitStatus(signalExitCode(*status.Signal))
	}
	return interp.ExitStatus(1)
}

func externalStartError(
	ctx context.Context,
	handler interp.HandlerContext,
	cmd *exec.Cmd,
	path string,
	err error,
) error {
	if errors.Is(err, syscall.ENOEXEC) {
		handler.Env = expand.ListEnviron(cmd.Env...)
		return handler.RunScript(ctx, path, cmd.Args)
	}
	_, _ = fmt.Fprintln(handler.Stderr, err)
	return interp.ExitStatus(126)
}
