package idlewatch

import (
	"context"
	"log/slog"
	"time"

	"github.com/wspl/demi/internal/gates"
)

// Activity is the demand a resource has now and when its last demand ended.
// A zero LastDemandEnd means no demand has ended yet.
type Activity struct {
	Busy          bool
	LastDemandEnd time.Time
}

// Of reads demand from a gate; maintenance does not count as demand.
func Of(gate gates.State) Activity {
	return Activity{Busy: gate.Demand > 0, LastDemandEnd: gate.LastDemandEnd}
}

// And combines resources, using the later demand end.
func (a Activity) And(other Activity) Activity {
	a.Busy = a.Busy || other.Busy
	if other.LastDemandEnd.After(a.LastDemandEnd) {
		a.LastDemandEnd = other.LastDemandEnd
	}
	return a
}

// Retirement holds a resource's reservation until Release, even if Retire
// never starts. Release must be idempotent and must not block.
type Retirement interface {
	Retire(ctx context.Context) error
	Release()
}

// Policy reads, reserves and retires a resource without exposing its identity.
type Policy interface {
	Check(ctx context.Context) (Activity, error)
	// Reserve returns nil while work or maintenance holds the resource.
	// An error returns no retirement; the policy releases partial acquisitions.
	Reserve(ctx context.Context) (Retirement, error)
	// Changed subscribes to changes that may let a reservation succeed. The
	// watch subscribes before reserving so a concurrent release is not missed.
	Changed() <-chan struct{}
}

// Watch runs until retirement succeeds or ctx is canceled. Its owner starts
// and joins its goroutine. Reads and failed retirements retry after poll;
// maintenance delays retirement without restarting window. Both durations
// are supplied by the owner, which shares one idle-window setting.
// Once retirement starts, cancellation cannot interrupt it: Watch waits for
// it to finish and release its reservation before returning. A retirement
// panic ends this watch, as a failed Rust retirement task does.
func Watch(ctx context.Context, policy Policy, window, poll time.Duration) {
	var since time.Time
	for ctx.Err() == nil {
		activity, err := policy.Check(ctx)
		if err != nil {
			slog.WarnContext(ctx, "an idle watch could not read its resource", "error", err)
			wait(ctx, poll, nil)
			continue
		}
		now := time.Now()
		since = idleStart(since, activity, now)
		if since.IsZero() {
			wait(ctx, poll, nil)
			continue
		}
		if remaining := since.Add(window).Sub(now); remaining > 0 {
			wait(ctx, min(remaining, poll), nil)
			continue
		}
		changed := policy.Changed()
		retirement, err := policy.Reserve(ctx)
		if err != nil {
			slog.WarnContext(ctx, "an idle watch could not reserve its resource", "error", err)
			wait(ctx, poll, nil)
			continue
		}
		if retirement == nil {
			wait(ctx, poll, changed)
			continue
		}
		outcome, next := reserved(ctx, policy, retirement, since)
		since = next
		if outcome == finished {
			return
		}
		if outcome == retryLater {
			wait(ctx, poll, nil)
		}
	}
}

type outcome uint8

const (
	recheck outcome = iota
	retryLater
	finished
)

// reserved owns the reservation through the second reading and retirement.
func reserved(ctx context.Context, policy Policy, retirement Retirement, since time.Time) (outcome, time.Time) {
	defer retirement.Release()
	again, err := policy.Check(ctx)
	if err != nil {
		slog.WarnContext(ctx, "an idle watch could not read its reserved resource", "error", err)
		return retryLater, since
	}
	if again.Busy || again.LastDemandEnd.After(since) {
		return recheck, idleStart(since, again, time.Now())
	}
	if ctx.Err() != nil {
		return finished, since
	}
	if retire(context.WithoutCancel(ctx), retirement) {
		return finished, since
	}
	return retryLater, since
}

// retire isolates a retirement task's panic from the owning component.
func retire(ctx context.Context, retirement Retirement) (done bool) {
	done = true
	defer func() {
		if value := recover(); value != nil {
			slog.ErrorContext(ctx, "an idle retirement panicked", "panic", value)
		}
	}()
	if err := retirement.Retire(ctx); err != nil {
		slog.WarnContext(ctx, "a retirement failed and is tried again later", "error", err)
		return false
	}
	return true
}

// idleStart keeps an idle window no earlier than the last demand end.
func idleStart(since time.Time, activity Activity, now time.Time) time.Time {
	if activity.Busy {
		return time.Time{}
	}
	if since.IsZero() {
		since = now
	}
	if activity.LastDemandEnd.After(since) {
		since = activity.LastDemandEnd
	}
	return since
}

// wait owns the watch's timer until its deadline, notification or cancellation.
func wait(ctx context.Context, delay time.Duration, changed <-chan struct{}) {
	timer := time.NewTimer(delay)
	defer timer.Stop()
	select {
	case <-ctx.Done():
	case <-changed:
	case <-timer.C:
	}
}
