// Package process owns the retry policy for processes started by the runner.
package process

import (
	"context"
	"errors"
	"syscall"
	"time"

	"github.com/wspl/demi/go/commandservice"
)

// StartRetry decides whether a failed start can be retried. Its zero value is
// ready to use. Descriptor exhaustion has no time limit; executable-busy waits
// have a cumulative one-second budget, independent of descriptor waits.
type StartRetry struct {
	backoff commandservice.Backoff
	busy    time.Duration
}

// Pause returns the delay before another attempt, or false for a final error.
func (r *StartRetry) Pause(err error) (time.Duration, bool) {
	if commandservice.Exhausted(err) {
		return r.backoff.Pause(), true
	}
	if !errors.Is(err, syscall.ETXTBSY) || r.busy >= time.Second {
		return 0, false
	}
	// Cap the last pause as well, so the total wait never exceeds the budget.
	pause := min(r.backoff.Pause(), time.Second-r.busy)
	r.busy += pause
	return pause, true
}

// Start retries an attempt that starts one process. The attempt must release
// resources on failure and return immediately after a successful start; waiting
// for the child is the caller's responsibility. Cancellation interrupts waits.
func Start[T any](ctx context.Context, attempt func() (T, error)) (T, error) {
	var zero T
	var retry StartRetry
	for {
		if err := ctx.Err(); err != nil {
			return zero, err
		}
		value, err := attempt()
		if err == nil {
			return value, nil
		}
		pause, again := retry.Pause(err)
		if !again {
			return value, err
		}
		timer := time.NewTimer(pause)
		select {
		case <-ctx.Done():
			timer.Stop()
			return zero, ctx.Err()
		case <-timer.C:
		}
	}
}
