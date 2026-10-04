package cmdsdk

import (
	"context"
	"sync/atomic"
	"time"
)

var pauses atomic.Uint64

// DescriptorPauses exposes retry observations for cmdsdktest.Pauses.
func DescriptorPauses() uint64 {
	return pauses.Load()
}

// Backoff spaces descriptor retries from 5 ms up to 100 ms; its zero value is ready.
type Backoff struct{ delay time.Duration }

// Pause returns and advances the next delay.
func (b *Backoff) Pause() time.Duration {
	if b.delay == 0 {
		b.delay = 5 * time.Millisecond
	}
	d := b.delay
	b.delay = min(d*2, 100*time.Millisecond)
	pauses.Add(1)
	return d
}

// Wait waits for the next descriptor retry or cancellation.
func (b *Backoff) Wait(ctx context.Context) error {
	timer := time.NewTimer(b.Pause())
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-timer.C:
		return nil
	}
}

// Retry waits out descriptor exhaustion. Cancellation returns the last open-file error.
func Retry[T any](ctx context.Context, attempt func() (T, error)) (T, error) {
	var b Backoff
	for {
		v, err := attempt()
		if !Exhausted(err) {
			return v, err
		}
		if b.Wait(ctx) != nil {
			return v, err
		}
	}
}
