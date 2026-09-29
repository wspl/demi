package commandservice

import (
	"errors"
	"time"
)

// Waiting out a lack of open files. A process keeps the limit the system gives
// it (docs/execution/runner.md § Load): an operation that finds no descriptor
// left waits and tries again, since a pipe, connection or file elsewhere will
// close, instead of failing.

// The pauses between attempts: nothing tells a process when a descriptor
// closes, so attempts are spaced from firstPause, doubling to at most lastPause.
const (
	firstPause = 5 * time.Millisecond
	lastPause  = 100 * time.Millisecond
)

// Exhausted reports whether err, or an error it wraps, means no descriptor was
// left.
func Exhausted(err error) bool {
	return errors.Is(err, errNoDescriptor)
}

// Backoff spaces descriptor and process-start retries. Its zero value is ready
// to use; callers own the wait and its cancellation.
type Backoff struct{ pause time.Duration }

// Pause returns the next delay, from 5 ms doubling to at most 100 ms.
func (b *Backoff) Pause() time.Duration {
	if b.pause == 0 {
		b.pause = firstPause
	}
	pause := b.pause
	b.pause = min(pause*2, lastPause)
	return pause
}

// RetryBlocking runs attempt until it succeeds or fails for another reason than
// a lack of open files, sleeping between attempts. It runs on a goroutine that
// may block.
func RetryBlocking[T any](attempt func() (T, error)) (T, error) {
	var backoff Backoff
	for {
		value, err := attempt()
		if err == nil || !Exhausted(err) {
			return value, err
		}
		time.Sleep(backoff.Pause())
	}
}
