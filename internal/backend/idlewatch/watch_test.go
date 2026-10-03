package idlewatch_test

import (
	"context"
	"errors"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/backend/idlewatch"
	"github.com/wspl/demi/internal/gates"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

const (
	window = time.Hour
	poll   = 30 * time.Second
)

// These in-process scenarios cost less than one second each, use virtual
// time only, and join every watch before auditing its reservation.
type gated struct {
	gate    *gates.Activity
	before  <-chan struct{}
	retire  func(context.Context) error
	check   func() error
	reserve func() error
}

func (g *gated) Check(_ context.Context) (idlewatch.Activity, error) {
	if g.check != nil {
		if err := g.check(); err != nil {
			return idlewatch.Activity{}, err
		}
	}
	return idlewatch.Of(g.gate.State()), nil
}

func (g *gated) Changed() <-chan struct{} { return g.gate.State().Changed() }

func (g *gated) Reserve(ctx context.Context) (idlewatch.Retirement, error) {
	if g.before != nil {
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-g.before:
		}
		g.before = nil
	}
	if g.reserve != nil {
		if err := g.reserve(); err != nil {
			return nil, err
		}
	}
	reservation := g.gate.TryReserve()
	if reservation == nil {
		return nil, nil
	}
	return &retirement{Reservation: reservation, run: g.retire}, nil
}

type retirement struct {
	*gates.Reservation
	run func(context.Context) error
}

func (r *retirement) Retire(ctx context.Context) error { return r.run(ctx) }

// counted records observable retirement attempts at their virtual timestamps.
func counted() (*gated, <-chan time.Time) {
	attempts := make(chan time.Time, 8)
	return &gated{gate: gates.NewActivity(nil), retire: func(context.Context) error {
		attempts <- time.Now()
		return nil
	}}, attempts
}

// start gives the test ownership of cancellation, joining and a lease audit.
func start(t *testing.T, g *gated) (context.CancelFunc, <-chan struct{}) {
	t.Helper()
	ctx, cancel := context.WithCancel(t.Context())
	done := make(chan struct{})
	go func() {
		defer close(done)
		idlewatch.Watch(ctx, g, window, poll)
	}()
	t.Cleanup(func() {
		cancel()
		<-done
		if s := g.gate.State(); s.Reserved || s.Demand != 0 || s.Maintenance != 0 {
			t.Errorf("watch left admission held: %+v", s)
		}
	})
	synctest.Wait()
	return cancel, done
}

func TestMaintenancePostponesRetirementUntilRelease(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		g, attempts := counted()
		held := g.gate.TryEnter(gates.Maintenance)
		defer held.Release()
		start(t, g)
		time.Sleep(2 * window)
		synctest.Wait()
		if len(attempts) != 0 {
			t.Fatal("maintenance was retired")
		}
		released := time.Now()
		held.Release()
		if got := <-attempts; got != released {
			t.Fatalf("retired at %v, want release %v", got, released)
		}
		synctest.Wait()
		if len(attempts) != 0 {
			t.Fatal("unexpected additional retirement")
		}
	})
}

func TestDemandRestartsFullWindow(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		g, attempts := counted()
		start(t, g)
		time.Sleep(window / 2)
		held := g.gate.TryEnter(gates.Demand)
		defer held.Release()
		time.Sleep(2 * window)
		synctest.Wait()
		if len(attempts) != 0 {
			t.Fatal("demand was retired")
		}
		ended := time.Now()
		held.Release()
		if elapsed := (<-attempts).Sub(ended); elapsed < window || elapsed > window+poll {
			t.Fatalf("retired after %v, want full window", elapsed)
		}
		synctest.Wait()
		if len(attempts) != 0 {
			t.Fatal("unexpected additional retirement")
		}
	})
}

func TestCanceledWatchFinishesRetirementWhileOtherWatchesContinue(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		slow, _ := counted()
		finish := make(chan struct{})
		var once sync.Once
		release := func() { once.Do(func() { close(finish) }) }
		// Deferred release also unblocks retirement before cleanup joins it.
		began := make(chan struct{})
		slow.retire = func(ctx context.Context) error {
			close(began)
			<-finish
			if ctx.Err() != nil {
				t.Error("started retirement was canceled")
			}
			return nil
		}
		cancel, done := start(t, slow)
		defer release()
		other, attempts := counted()
		start(t, other)
		<-began
		cancel()
		<-attempts
		synctest.Wait()
		select {
		case <-done:
			t.Fatal("owner joined before retirement finished")
		default:
		}
		release()
		<-done
		synctest.Wait()
		if len(attempts) != 0 {
			t.Fatal("unexpected additional retirement")
		}
	})
}

func TestFailedRetirementReleasesAndRetriesAfterPoll(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		g, _ := counted()
		attempts := make(chan time.Time, 2)
		g.retire = func(context.Context) error {
			attempts <- time.Now()
			return errors.New("release failed")
		}
		start(t, g)
		first := <-attempts
		synctest.Wait()
		if g.gate.State().Reserved {
			t.Fatal("failed retirement retained reservation")
		}
		if elapsed := (<-attempts).Sub(first); elapsed != poll {
			t.Fatalf("retry after %v, want %v", elapsed, poll)
		}
		synctest.Wait()
		if len(attempts) != 0 {
			t.Fatal("unexpected additional retirement")
		}
	})
}

func TestBriefDemandDuringReservationRestartsWindow(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		g, attempts := counted()
		proceed := make(chan struct{})
		g.before = proceed
		start(t, g)
		time.Sleep(window + time.Second)
		held := g.gate.TryEnter(gates.Demand)
		defer held.Release()
		ended := time.Now()
		held.Release()
		close(proceed)
		if elapsed := (<-attempts).Sub(ended); elapsed < window {
			t.Fatalf("retired after only %v", elapsed)
		}
		synctest.Wait()
		if len(attempts) != 0 {
			t.Fatal("unexpected additional retirement")
		}
	})
}

func TestReadAndReserveFailuresWaitBeforeRetry(t *testing.T) {
	for _, stage := range []string{"read", "reserve", "reserved read"} {
		t.Run(stage, func(t *testing.T) {
			synctest.Test(t, func(t *testing.T) {
				g, attempts := counted()
				failure := errors.New("resource unavailable")
				failed := make(chan time.Time, 1)
				didFail := false
				failOnce := func() error {
					if didFail {
						return nil
					}
					didFail = true
					failed <- time.Now()
					return failure
				}
				if stage == "reserve" {
					g.reserve = failOnce
				} else {
					g.check = func() error {
						if stage == "reserved read" && !g.gate.State().Reserved {
							return nil
						}
						return failOnce()
					}
				}
				start(t, g)
				at := <-failed
				synctest.Wait()
				if g.gate.State().Reserved {
					t.Fatal("failed check retained reservation")
				}
				got := <-attempts
				want := at.Add(poll)
				if stage == "read" {
					want = want.Add(window)
				}
				if got != want {
					t.Fatalf("retired at %v, want %v", got, want)
				}
			})
		})
	}
}

func TestRetirementPanicEndsWatchAndReleasesReservation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		g, _ := counted()
		g.retire = func(context.Context) error { panic("retirement failed") }
		_, done := start(t, g)
		<-done
		if g.gate.State().Reserved {
			t.Fatal("panicked retirement retained reservation")
		}
	})
}

func TestCancellationReleasesUnstartedReservation(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		g, attempts := counted()
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		g.check = func() error {
			if g.gate.State().Reserved {
				cancel()
			}
			return nil
		}
		done := make(chan struct{})
		go func() {
			defer close(done)
			idlewatch.Watch(ctx, g, window, poll)
		}()
		<-done
		if len(attempts) != 0 || g.gate.State().Reserved {
			t.Fatal("canceled watch retired or retained reservation")
		}
	})
}

func TestCombinedActivityUsesLatestDemand(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		first := gates.NewActivity(nil)
		second := gates.NewActivity(nil)
		held := first.TryEnter(gates.Demand)
		defer held.Release()
		other := second.TryEnter(gates.Demand)
		defer other.Release()
		other.Release()
		combined := idlewatch.Of(first.State()).And(idlewatch.Of(second.State()))
		if !combined.Busy {
			t.Fatal("one gate's demand did not keep resource busy")
		}
		time.Sleep(time.Second)
		held.Release()
		combined = idlewatch.Of(first.State()).And(idlewatch.Of(second.State()))
		if combined.Busy || combined.LastDemandEnd != time.Now() {
			t.Fatalf("combined activity lost latest end: %+v", combined)
		}
	})
}
