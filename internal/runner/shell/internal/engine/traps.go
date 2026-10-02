package engine

import (
	"context"
	"fmt"
	"slices"
	"strconv"
	"strings"

	"mvdan.cc/sh/v3/interp"
	"mvdan.cc/sh/v3/syntax"
)

// trap records signal handlers without installing process-wide signal handlers.
func (e *execution) trap(ctx context.Context, args []string) error {
	hc := interp.HandlerCtx(ctx)
	state := hc.Scope().(*interpreterScope)
	args = args[1:]
	if len(args) > 0 && args[0] == "-l" {
		_, err := fmt.Fprintln(hc.Stdout, signalNames)
		return err
	}
	listing := len(args) == 0
	if len(args) > 0 && args[0] == "-p" {
		listing = true
		args = args[1:]
	}
	if len(args) > 0 && args[0] == "--" {
		args = args[1:]
	}
	if listing {
		names := args
		if len(names) == 0 {
			for name := range state.traps {
				names = append(names, name)
			}
			slices.Sort(names)
		}
		for _, name := range names {
			normalized, ok := trapSignal(name)
			if !ok {
				return diagnostic(ctx, 1, "trap: %s: invalid signal specification\n", name)
			}
			if handler, exists := state.traps[normalized]; exists {
				quoted, err := syntax.Quote(handler, syntax.LangBash)
				if err != nil {
					return err
				}
				if _, err := fmt.Fprintf(hc.Stdout, "trap -- %s %s\n", quoted, normalized); err != nil {
					return err
				}
			}
		}
		return nil
	}
	handler := "-"
	if len(args) > 1 {
		handler, args = args[0], args[1:]
	}
	for _, name := range args {
		normalized, ok := trapSignal(name)
		if !ok {
			return diagnostic(ctx, 1, "trap: %s: invalid signal specification\n", name)
		}
		if state.traps == nil {
			state.traps = make(map[string]string)
		}
		if handler == "-" {
			delete(state.traps, normalized)
		} else {
			state.traps[normalized] = handler
		}
		if normalized == "EXIT" || normalized == "ERR" {
			if err := hc.NativeBuiltin(ctx, []string{"trap", "--", handler, normalized}); err != nil {
				return err
			}
		}
	}
	return nil
}

// Signal names are shell syntax even on a host without Unix signal delivery.
const signalNames = "EXIT HUP INT QUIT ILL TRAP ABRT IOT BUS EMT FPE KILL USR1 SEGV USR2 PIPE ALRM TERM STKFLT CHLD CLD CONT STOP TSTP TTIN TTOU URG XCPU XFSZ VTALRM PROF WINCH IO POLL PWR SYS INFO LOST UNUSED ERR DEBUG RETURN"

func trapSignal(name string) (string, bool) {
	name = strings.TrimPrefix(name, "SIG")
	if name == "0" {
		return "EXIT", true
	}
	if number, err := strconv.Atoi(name); err == nil {
		return numericSignal(number)
	}
	if slices.Contains(strings.Fields(signalNames), name) {
		return name, true
	}
	for _, prefix := range []string{"RTMIN", "RTMAX"} {
		if name == prefix {
			return name, true
		}
		offset := strings.TrimPrefix(name, prefix)
		if offset != name && len(offset) > 1 && (offset[0] == '+' || offset[0] == '-') {
			number, err := strconv.Atoi(offset[1:])
			if err == nil && number <= 30 {
				return name, true
			}
		}
	}
	return "", false
}
