package shell

import (
	"context"
	"errors"
	"os"
	"syscall"
	"time"
)

// waitForDescriptors calls open until it does not fail for lack of a file
// descriptor, or until ctx ends. Out of open files, a job's pipes,
// redirections and programs wait for one to close instead of failing
// (docs/demi-next/runner.md § Load). Nothing tells the process when one
// closes, so attempts are spaced from 5 ms, doubling to at most 100 ms.
//
// TODO(N1): internal/commandservice owns waiting out a lack of open files
// (docs/package-boundaries.md); use its implementation once it lands.
func waitForDescriptors[T any](ctx context.Context, open func() (T, error)) (T, error) {
	pause := 5 * time.Millisecond
	for {
		value, err := open()
		if !errors.Is(err, syscall.EMFILE) && !errors.Is(err, syscall.ENFILE) {
			return value, err
		}
		timer := time.NewTimer(pause)
		select {
		case <-ctx.Done():
			timer.Stop()
			return value, err
		case <-timer.C:
		}
		pause = min(2*pause, 100*time.Millisecond)
	}
}

// osPipe creates an OS pipe, waiting for descriptors.
func osPipe(ctx context.Context) (r, w *os.File, err error) {
	ends, err := waitForDescriptors(ctx, func() ([2]*os.File, error) {
		r, w, err := os.Pipe()
		return [2]*os.File{r, w}, err
	})
	return ends[0], ends[1], err
}
