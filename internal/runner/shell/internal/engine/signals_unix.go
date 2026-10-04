//go:build darwin || linux

package engine

import (
	"context"
	"errors"
	"fmt"
	"os"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerproto"
	"golang.org/x/sys/unix"
	"mvdan.cc/sh/v3/interp"
)

func (e *execution) kill(ctx context.Context, args []string) error {
	signal := unix.SIGTERM
	args = args[1:]
	if len(args) > 0 && args[0] == "-l" {
		_, err := fmt.Fprintln(
			interp.HandlerCtx(ctx).Stdout,
			"HUP INT QUIT ILL TRAP ABRT BUS FPE KILL USR1 SEGV USR2 PIPE ALRM TERM CHLD CONT STOP TSTP TTIN TTOU",
		)
		return err
	}
	args, signal, err := killOptions(ctx, args, signal)
	if err != nil {
		return err
	}
	if len(args) == 0 {
		return diagnostic(ctx, 2, "kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ...\n")
	}
	for _, arg := range args {
		if strings.HasPrefix(arg, "g") {
			if err := killBackground(ctx, arg, signal); err != nil {
				return err
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

func signalExitCode(name string) uint8 {
	return uint8(128 + unix.SignalNum(name))
}

// signal sends to active processes descended from this shell scope. The job
// retains only active commands, not every scope ever made by a long-running loop.
func (s *interpreterScope) signal(signal runnerproto.Signal) error {
	s.owner.mu.Lock()
	var commands []*process.Command
	for command, scope := range s.owner.commands {
		for ancestor := scope; ancestor != nil; ancestor = ancestor.parent {
			if ancestor == s {
				commands = append(commands, command)
				break
			}
		}
	}
	s.owner.mu.Unlock()
	var err error
	for _, command := range commands {
		err = errors.Join(err, command.Signal(signal))
	}
	return err
}

func killOptions(ctx context.Context, args []string, signal unix.Signal) ([]string, unix.Signal, error) {
	for len(args) > 0 && strings.HasPrefix(args[0], "-") {
		arg := args[0]
		args = args[1:]
		if arg == "--" {
			break
		}
		if arg == "-s" || arg == "-n" {
			if len(args) == 0 {
				return nil, signal, diagnostic(ctx, 2, "kill: %s: option requires an argument\n", arg)
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
				return nil, signal, diagnostic(ctx, 1, "kill: %s: invalid signal specification\n", arg)
			}
		}
	}
	return args, signal, nil
}

func killBackground(ctx context.Context, arg string, signal unix.Signal) error {
	scope, _, err := interp.HandlerCtx(ctx).BackgroundScope(arg)
	if err != nil {
		return diagnostic(ctx, 1, "kill: %v\n", err)
	}
	if signal == 0 {
		return nil
	}
	if err := scope.(*interpreterScope).signal(runnerproto.Signal(unix.SignalName(signal))); err != nil {
		return diagnostic(ctx, 1, "kill: %v\n", err)
	}
	switch signal {
	case unix.SIGINT, unix.SIGTERM, unix.SIGKILL, unix.SIGHUP, unix.SIGQUIT:
		if err := interp.HandlerCtx(ctx).TerminateBackground(arg, uint8(128+signal)); err != nil {
			return err
		}
	}
	return nil
}
