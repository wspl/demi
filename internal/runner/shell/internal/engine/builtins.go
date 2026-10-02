package engine

import (
	"context"
	"fmt"
	"strings"

	"mvdan.cc/sh/v3/interp"
)

// builtins replaces operations on the runner process and registers declared roots.
func (e *execution) builtins() map[string]interp.ExecHandlerFunc {
	handlers := map[string]interp.ExecHandlerFunc{"trap": e.trap, "times": e.times, "exec": e.execBuiltin, "umask": e.umask, "ulimit": e.ulimit, "kill": e.kill, "suspend": refuseSuspend, "fg": refuseJobControl, "bg": refuseJobControl}
	if e.options.Commands != nil {
		for _, root := range e.options.Commands.Roots {
			handlers[root] = e.declared
		}
	}
	return handlers
}
func refuseSuspend(ctx context.Context, args []string) error {
	return diagnostic(ctx, 1, "%s: a job cannot suspend the runner it runs in\n", args[0])
}
func refuseJobControl(ctx context.Context, args []string) error {
	return diagnostic(ctx, 1, "%s: no job control\n", args[0])
}

// diagnostic writes the builtin's diagnostic and returns a shell status, not a fatal job error.
func diagnostic(ctx context.Context, code uint8, format string, args ...any) error {
	if _, err := fmt.Fprintf(interp.HandlerCtx(ctx).Stderr, format, args...); err != nil {
		return err
	}
	return interp.ExitStatus(code)
}

type execKey struct{}
type execOptions struct {
	argv0 string
	empty bool
}

func (e *execution) execBuiltin(ctx context.Context, args []string) error {
	hc := interp.HandlerCtx(ctx)
	options := execOptions{}
	login := false
	args = args[1:]
	for len(args) > 0 {
		arg := args[0]
		if arg == "--" {
			args = args[1:]
			break
		}
		if !strings.HasPrefix(arg, "-") {
			break
		}
		args = args[1:]

		flags := arg[1:]
		for len(flags) > 0 {
			flag := flags[0]
			flags = flags[1:]
			switch flag {
			case 'c':
				options.empty = true
			case 'l':
				login = true
			case 'a':
				if flags != "" {
					options.argv0, flags = flags, ""
				} else {
					if len(args) == 0 {
						return diagnostic(ctx, 2, "exec: -a: option requires an argument\n")
					}
					options.argv0, args = args[0], args[1:]
				}
			default:
				return diagnostic(ctx, 2, "exec: -%c: invalid option\n", flag)
			}
		}
	}
	if len(args) == 0 {
		return hc.NativeBuiltin(ctx, []string{"exec"})
	}
	if login {
		if options.argv0 == "" {
			options.argv0 = args[0]
		}
		options.argv0 = "-" + options.argv0
	}
	ctx = context.WithValue(ctx, execKey{}, options)
	err := hc.NativeBuiltin(ctx, append([]string{"command"}, args...))
	return hc.Exit(err)
}
