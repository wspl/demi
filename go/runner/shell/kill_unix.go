//go:build unix

package shell

import (
	"context"
	"errors"
	"fmt"
	"strconv"
	"strings"

	"golang.org/x/sys/unix"
	"mvdan.cc/sh/v3/interp"
)

// kill follows brush's signal parsing and usage diagnostics after the runner
// self-target guard. Its default is SIGKILL, rather than the Host kill's SIGTERM.
func kill(ctx context.Context, args []string) error {
	signal := unix.SIGKILL
	virtual := ""
	list := false
	var name *string
	var number *uint64
	var operands []string
	seen := make(map[string]bool)
	for i := 1; i < len(args); i++ {
		arg := args[i]
		if arg == "-L" {
			arg = "-l"
		}
		label := map[string]string{"-s": "-s <SIG_NAME>", "-n": "-n <SIG_NUM>", "-l": "-l"}[arg]
		if label != "" {
			if seen[arg] {
				return builtinUsage(ctx, fmt.Sprintf("error: the argument '%s' cannot be used multiple times\n\nUsage: kill [OPTIONS] [ARGS]...\n\nFor more information, try '--help'.\n\n", label))
			}
			seen[arg] = true
		}
		switch arg {
		case "-h", "--help":
			return builtinUsage(ctx, killHelp+"\n")
		case "-l":
			list = true
		case "-s", "-n":
			if i+1 == len(args) {
				return builtinUsage(ctx, fmt.Sprintf("error: a value is required for '%s' but none was supplied\n\nFor more information, try '--help'.\n\n", label))
			}
			i++
			value := args[i]
			if arg == "-s" {
				name = &value
				continue
			}
			n, err := strconv.ParseUint(strings.TrimPrefix(value, "+"), 10, 64)
			if err != nil {
				reason := "invalid digit found in string"
				if value == "" {
					reason = "cannot parse integer from empty string"
				}
				if errors.Is(err, strconv.ErrRange) {
					reason = "number too large to fit in target type"
				}
				return builtinUsage(ctx, fmt.Sprintf("error: invalid value '%s' for '-n <SIG_NUM>': %s\n\nFor more information, try '--help'.\n\n", value, reason))
			}
			number = &n
		case "--":
			operands = append(operands, args[i+1:]...)
			i = len(args)
		default:
			operands = append(operands, arg)
		}
	}
	if name != nil {
		var valid bool
		signal, virtual, valid = killSignal(*name)
		if !valid {
			return usage(ctx, "kill", "invalid signal name: "+*name)
		}
	}
	if number != nil {
		signal = unix.Signal(int32(*number))
		virtual = ""
		if signal == 0 {
			virtual = "EXIT"
		} else if unix.SignalName(signal) == "" {
			return usage(ctx, "kill", fmt.Sprintf("invalid signal number: %d", *number))
		}
	}
	var target string
	for _, arg := range operands {
		if value, ok := strings.CutPrefix(arg, "-"); ok {
			var valid bool
			signal, virtual, valid = killSignal(value)
			if !valid {
				return usage(ctx, "kill", "invalid signal name")
			}
		} else if target == "" {
			target = arg
		} else {
			return usage(ctx, "kill", "too many jobs or processes specified")
		}
	}
	if list {
		if target == "" {
			first := true
			for number := 1; number < 32; number++ {
				name := unix.SignalName(unix.Signal(number))
				if name == "" {
					continue
				}
				if !first {
					if _, err := fmt.Fprintln(interp.HandlerCtx(ctx).Stdout); err != nil {
						return err
					}
				}
				if _, err := fmt.Fprintf(interp.HandlerCtx(ctx).Stdout, "%d) %s", number, name); err != nil {
					return err
				}
				first = false
			}
			return nil
		}
		number, err := strconv.Atoi(target)
		if err == nil {
			name := unix.SignalName(unix.Signal(number))
			if number == 0 {
				name = "EXIT"
			}
			if name == "" {
				if _, err := fmt.Fprintf(interp.HandlerCtx(ctx).Stderr, "%s: invalid signal specification\n", target); err != nil {
					return err
				}
				return interp.ExitStatus(1)
			}
			_, err = fmt.Fprintln(interp.HandlerCtx(ctx).Stdout, strings.TrimPrefix(name, "SIG"))
			return err
		}
		name := strings.ToUpper(target)
		if name == "DEBUG" || name == "ERR" || name == "RETURN" {
			_, err = fmt.Fprintln(interp.HandlerCtx(ctx).Stdout, name)
			return err
		}
		number = int(unix.SignalNum("SIG" + strings.TrimPrefix(name, "SIG")))
		if number == 0 && name != "EXIT" {
			if _, err := fmt.Fprintf(interp.HandlerCtx(ctx).Stderr, "%s: invalid signal specification\n", target); err != nil {
				return err
			}
			return interp.ExitStatus(1)
		}
		_, err = fmt.Fprintln(interp.HandlerCtx(ctx).Stdout, number)
		return err
	}
	if target == "" {
		return usage(ctx, "kill", "invalid usage")
	}
	if strings.HasPrefix(target, "%") {
		return diagnostic(ctx, "kill", fmt.Errorf("%s: no such job", target))
	}
	pid, err := strconv.Atoi(target)
	if err != nil {
		return diagnostic(ctx, "kill", err)
	}
	if virtual != "" {
		return diagnostic(ctx, "kill", fmt.Errorf("%s: invalid signal specification", virtual))
	}
	if err := unix.Kill(pid, signal); err != nil {
		return diagnostic(ctx, "kill", fmt.Errorf("failed to send signal"))
	}
	return nil
}

const killHelp = "Signal a job or process\n\nUsage: kill [OPTIONS] [ARGS]...\n\nArguments:\n  [ARGS]...  \n\nOptions:\n  -s <SIG_NAME>  Name of the signal to send\n  -n <SIG_NUM>   Number of the signal to send\n  -l             List known signal names\n  -h, --help     Print help\n"

// killSignal accepts brush's real signals and shell-only trap names.
func killSignal(name string) (unix.Signal, string, bool) {
	name = strings.ToUpper(name)
	switch name {
	case "EXIT", "DEBUG", "ERR", "RETURN":
		return 0, name, true
	}
	signal := unix.SignalNum("SIG" + strings.TrimPrefix(name, "SIG"))
	return signal, "", signal != 0
}
