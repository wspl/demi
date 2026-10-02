//go:build darwin || linux

package shell

import (
	"context"
	"fmt"
	"os"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/runnerwire"
	"golang.org/x/sys/unix"
	"mvdan.cc/sh/v3/interp"
)

func (e *execution) kill(ctx context.Context, args []string) error {
	signal := unix.SIGTERM
	args = args[1:]
	if len(args) > 0 && args[0] == "-l" {
		_, err := fmt.Fprintln(interp.HandlerCtx(ctx).Stdout, "HUP INT QUIT ILL TRAP ABRT BUS FPE KILL USR1 SEGV USR2 PIPE ALRM TERM CHLD CONT STOP TSTP TTIN TTOU")
		return err
	}
	for len(args) > 0 && strings.HasPrefix(args[0], "-") {
		arg := args[0]
		args = args[1:]
		if arg == "--" {
			break
		}
		if arg == "-s" || arg == "-n" {
			if len(args) == 0 {
				return diagnostic(ctx, 2, "kill: %s: option requires an argument\n", arg)
			}
			arg = args[0]
			args = args[1:]
		} else {
			arg = strings.TrimPrefix(arg, "-")
		}
		parsed, err := strconv.Atoi(arg)
		if err == nil {
			signal = unix.Signal(parsed)
		} else {
			signal = unix.SignalNum("SIG" + strings.TrimPrefix(arg, "SIG"))
			if signal == 0 {
				return diagnostic(ctx, 1, "kill: %s: invalid signal specification\n", arg)
			}
		}
	}
	if len(args) == 0 {
		return diagnostic(ctx, 2, "kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ...\n")
	}
	for _, arg := range args {
		if strings.HasPrefix(arg, "g") {
			scope, cancel, err := interp.HandlerCtx(ctx).BackgroundScope(arg)
			if err != nil {
				return diagnostic(ctx, 1, "kill: %v\n", err)
			}
			if signal == 0 {
				continue
			}
			if err := scope.(*interpreterScope).signal(runnerwire.Signal(unix.SignalName(signal))); err != nil {
				return diagnostic(ctx, 1, "kill: %v\n", err)
			}
			switch signal {
			case unix.SIGINT, unix.SIGTERM, unix.SIGKILL, unix.SIGHUP, unix.SIGQUIT:
				cancel()
			}
			continue
		}
		pid, err := strconv.Atoi(arg)
		if err != nil {
			return diagnostic(ctx, 1, "kill: %s: invalid process or job ID\n", arg)
		}
		if pid == 0 || pid == os.Getpid() {
			return diagnostic(ctx, 1, "kill: a job cannot signal the runner it runs in\n")
		}
		if err := unix.Kill(pid, signal); err != nil {
			return diagnostic(ctx, 1, "kill: (%s) - %v\n", arg, err)
		}
	}
	return nil
}
