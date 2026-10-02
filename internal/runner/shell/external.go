package shell

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
	hc := interp.HandlerCtx(ctx)
	state := hc.Scope().(*interpreterScope)
	state.Waiting(1)
	defer state.Waiting(-1)
	path, err := interp.LookPathDir(hc.Dir, hc.Env, args[0])
	if err != nil {
		_, _ = fmt.Fprintln(hc.Stderr, err)
		return interp.ExitStatus(127)
	}
	cmd := exec.Command(path, args[1:]...)
	cmd.Dir = hc.Dir
	cmd.Env = exported(hc.Env)
	if options, ok := ctx.Value(execKey{}).(execOptions); ok {
		if options.empty {
			cmd.Env = []string{}
		}
		if options.argv0 != "" {
			cmd.Args[0] = options.argv0
		}
	}
	cmd.Stdin = hc.Stdin
	cmd.Stdout = hc.Stdout
	cmd.Stderr = hc.Stderr
	if err = extraDescriptors(cmd, hc.Descriptors()); err != nil {
		return err
	}
	child := process.Wrap(cmd, true, state.attributes)
	if err = child.Start(ctx); err != nil {
		if errors.Is(err, syscall.ENOEXEC) {
			return hc.RunScript(ctx, path, args)
		}
		_, _ = fmt.Fprintln(hc.Stderr, err)
		return interp.ExitStatus(126)
	}
	release := state.register(child)
	defer release()
	status := child.Wait(ctx)
	if ctx.Err() != nil {
		return ctx.Err()
	}
	if status.Error != nil {
		return errors.New(*status.Error)
	}
	if status.Code != nil {
		if *status.Code == 0 {
			return nil
		}
		return interp.ExitStatus(uint8(*status.Code))
	}
	return interp.ExitStatus(1)
}

// exported supplies only variables marked for child inheritance.
func exported(env expand.Environ) []string {
	var result []string
	env.Each(func(name string, value expand.Variable) bool {
		if value.Exported && value.IsSet() {
			result = append(result, name+"="+value.String())
		}
		return true
	})
	return result
}
