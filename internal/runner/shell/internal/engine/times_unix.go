//go:build darwin || linux

package engine

import (
	"context"
	"fmt"

	"golang.org/x/sys/unix"
	"mvdan.cc/sh/v3/interp"
)

// times labels process-wide accounting because the OS cannot account goroutines per job.
func (e *execution) times(ctx context.Context, _ []string) error {
	var usage unix.Rusage
	if err := unix.Getrusage(unix.RUSAGE_SELF, &usage); err != nil {
		return err
	}
	_, err := fmt.Fprintf(interp.HandlerCtx(ctx).Stdout, "runner: %dm%.3fs %dm%.3fs\n", usage.Utime.Sec/60, float64(usage.Utime.Sec%60)+float64(usage.Utime.Usec)/1e6, usage.Stime.Sec/60, float64(usage.Stime.Sec%60)+float64(usage.Stime.Usec)/1e6)
	return err
}

func numericSignal(number int) (string, bool) {
	name := unix.SignalName(unix.Signal(number))
	if len(name) > 3 {
		return name[3:], true
	}
	if number >= 34 && number <= 64 {
		return fmt.Sprintf("RTMIN+%d", number-34), true
	}
	return "", false
}
