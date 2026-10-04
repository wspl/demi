package engine

import (
	"context"
	"fmt"

	"golang.org/x/sys/windows"
	"mvdan.cc/sh/v3/interp"
)

func (e *execution) umask(ctx context.Context, args []string) error {
	return interp.HandlerCtx(ctx).NativeBuiltin(ctx, args)
}

func (e *execution) ulimit(ctx context.Context, args []string) error {
	return interp.HandlerCtx(ctx).NativeBuiltin(ctx, args)
}

func (e *execution) kill(ctx context.Context, args []string) error {
	return interp.HandlerCtx(ctx).NativeBuiltin(ctx, args)
}

func (e *execution) times(ctx context.Context, _ []string) error {
	var created, exited, kernel, user windows.Filetime
	if err := windows.GetProcessTimes(windows.CurrentProcess(), &created, &exited, &kernel, &user); err != nil {
		return err
	}
	seconds := func(t windows.Filetime) float64 {
		return float64(uint64(t.HighDateTime)<<32|uint64(t.LowDateTime)) / 1e7
	}
	_, err := fmt.Fprintf(interp.HandlerCtx(ctx).Stdout, "runner: %.3fs %.3fs\n", seconds(user), seconds(kernel))
	return err
}

func numericSignal(number int) (string, bool) {
	names := []string{
		"",
		"HUP",
		"INT",
		"QUIT",
		"ILL",
		"TRAP",
		"ABRT",
		"BUS",
		"FPE",
		"KILL",
		"USR1",
		"SEGV",
		"USR2",
		"PIPE",
		"ALRM",
		"TERM",
	}
	if number > 0 && number < len(names) {
		return names[number], true
	}
	return "", false
}

func signalExitCode(_ string) uint8 {
	return 1
}
