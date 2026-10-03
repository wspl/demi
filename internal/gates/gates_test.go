package gates_test

import (
	"context"
	"errors"
	"reflect"
	"sync"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/gates/gatestest"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

// These in-process admission scenarios use only events and virtual time;
// each should take less than one second and owns no external resources.

// enter acquires an activity lease owned by the test, including failure cleanup.
func enter(t *testing.T, a *gates.Activity, purpose gates.Purpose) *gates.Lease {
	t.Helper()
	l, err := a.Enter(t.Context(), purpose)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(l.Release)
	return l
}

// acquire owns a serial turn for the test until explicitly released or cleanup.
func acquire(t *testing.T, s *gates.Serial) *gates.Permit {
	t.Helper()
	p, err := s.Acquire(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(p.Release)
	return p
}

// keyed owns a keyed serial turn, retaining failure-path cleanup.
func keyed(t *testing.T, s *gates.KeyedSerial[string], key string) *gates.Permit {
	t.Helper()
	p, err := s.Acquire(t.Context(), key)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(p.Release)
	return p
}

func TestReservationDrainsAndQueuesEntrants(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		a := gates.NewActivity(nil)
		first := enter(t, a, gates.Demand)
		second := enter(t, a, gates.Demand)
		if r := a.TryReserve(); r != nil {
			r.Release()
			t.Fatal("reserved while demand held the gate")
		}
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		writer := make(chan *gates.Reservation, 1)
		var workers sync.WaitGroup
		defer workers.Wait()
		defer first.Release()
		defer second.Release()
		defer cancel()
		workers.Go(func() {
			r, err := a.Reserve(ctx)
			if err != nil {
				return
			}
			t.Cleanup(r.Release)
			writer <- r
		})
		synctest.Wait()
		if l := a.TryEnter(gates.Demand); l != nil {
			l.Release()
			t.Fatal("entrant bypassed queued reservation")
		}
		entrants := make(chan *gates.Lease, 1)
		workers.Go(func() {
			l, err := a.Enter(ctx, gates.Demand)
			if err == nil {
				t.Cleanup(l.Release)
				entrants <- l
			}
		})
		synctest.Wait()
		if n, _ := gatestest.Waiting(a); n != 1 {
			t.Fatalf("waiting = %d, want 1", n)
		}
		first.Release()
		synctest.Wait()
		if len(writer) != 0 || len(entrants) != 0 {
			t.Fatal("admitted before current work drained")
		}
		second.Release()
		r := <-writer
		defer r.Release()
		if !a.State().Reserved || len(entrants) != 0 {
			t.Fatal("reservation did not exclude entrant")
		}
		r.Release()
		l := <-entrants
		defer l.Release()
		if a.State().Demand != 1 {
			t.Fatal("entrant not counted")
		}
		l.Release()
		if a.State().Demand != 0 {
			t.Fatal("released entrant still counted")
		}
	})
}

func TestCancelDrainingReservationAdmitsWaiters(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		a := gates.NewActivity(nil)
		held := enter(t, a, gates.Demand)
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		writer := make(chan error, 1)
		go func() {
			r, err := a.Reserve(ctx)
			if r != nil {
				r.Release()
			}
			writer <- err
		}()
		synctest.Wait()
		entrant := make(chan *gates.Lease, 1)
		go func() { entrant <- enter(t, a, gates.Demand) }()
		synctest.Wait()
		if len(entrant) != 0 {
			t.Fatal("entrant bypassed reservation")
		}
		cancel()
		if err := <-writer; !errors.Is(err, context.Canceled) {
			t.Fatalf("reservation error = %v", err)
		}
		l := <-entrant
		if a.State().Demand != 2 {
			t.Fatal("canceling reservation changed existing lease")
		}
		l.Release()
		held.Release()
		r := a.TryReserve()
		if r == nil {
			t.Fatal("idle gate did not reserve")
		}
		r.Release()
	})
}

func TestCancelEntrantNeverAdmits(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		a := gates.NewActivity(nil)
		r := a.TryReserve()
		if r == nil {
			t.Fatal("idle gate did not reserve")
		}
		defer r.Release()
		if extra := a.TryReserve(); extra != nil {
			extra.Release()
			t.Fatal("second reservation admitted")
		}
		if l := a.TryEnter(gates.Maintenance); l != nil {
			l.Release()
			t.Fatal("maintenance bypassed reservation")
		}
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		done := make(chan error, 1)
		_, changed := gatestest.Waiting(a)
		go func() {
			l, err := a.Enter(ctx, gates.Demand)
			if l != nil {
				l.Release()
			}
			done <- err
		}()
		<-changed
		synctest.Wait()
		n, changed := gatestest.Waiting(a)
		if n != 1 {
			t.Fatalf("waiting = %d, want 1", n)
		}
		cancel()
		if err := <-done; !errors.Is(err, context.Canceled) {
			t.Fatalf("entrant error = %v", err)
		}
		<-changed
		if n, _ := gatestest.Waiting(a); n != 0 {
			t.Fatalf("canceled entrant still waiting: %d", n)
		}
		r.Release()
		if s := a.State(); s.Demand != 0 || s.Reserved {
			t.Fatalf("canceled entrant changed state: %+v", s)
		}
		again := a.TryReserve()
		if again == nil {
			t.Fatal("canceled entrant retained permit")
		}
		again.Release()
	})
}

func TestStateCountsPurposesAndRetainsDemandEnd(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var hub gates.Hub
		bumps := hub.Subscribe()
		a := gates.NewActivity(&hub)
		original := a.State()
		maintenance := a.TryEnter(gates.Maintenance)
		if maintenance == nil {
			t.Fatal("maintenance refused on idle gate")
		}
		defer maintenance.Release()
		demand := enter(t, a, gates.Demand)
		second := enter(t, a, gates.Demand)
		if demand.Purpose() != gates.Demand || maintenance.Purpose() != gates.Maintenance {
			t.Fatal("lease lost its purpose")
		}
		<-original.Changed()
		<-bumps
		seen := a.State()
		if seen.Demand != 2 || seen.Maintenance != 1 || !seen.LastDemandEnd.IsZero() {
			t.Fatalf("state = %+v", seen)
		}
		demand.Release()
		if !a.State().LastDemandEnd.IsZero() {
			t.Fatal("demand end recorded while another demand lease remains")
		}
		time.Sleep(5 * time.Second) // Virtual time, not scheduling synchronization.
		ended := time.Now()
		bumps = hub.Subscribe()
		second.Release()
		<-bumps
		if a.State().LastDemandEnd != ended {
			t.Fatal("last demand end was not recorded")
		}
		time.Sleep(5 * time.Second)
		bumps = hub.Subscribe()
		maintenance.Release()
		<-bumps
		if s := a.State(); s.LastDemandEnd != ended || s.Maintenance != 0 {
			t.Fatalf("maintenance changed demand end: %+v", s)
		}
		if original.Demand != 0 || seen.Demand != 2 {
			t.Fatal("published snapshots mutated")
		}
		// A missed notification cannot erase an idle boundary; renewed demand
		// retains it until the next final release.
		next := enter(t, a, gates.Demand)
		if a.State().LastDemandEnd != ended {
			t.Fatal("renewed demand erased previous end")
		}
		next.Release()
		if a.State().LastDemandEnd != time.Now() {
			t.Fatal("renewed demand did not advance end")
		}
		bumps = hub.Subscribe()
		other := gates.NewActivity(&hub)
		l := enter(t, other, gates.Demand)
		<-bumps
		l.Release()
	})
}

func TestSerialFIFOAndHandoff(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var s gates.Serial
		first := acquire(t, &s)
		if p := s.TryAcquire(); p != nil {
			p.Release()
			t.Fatal("serial gate admitted a second holder")
		}
		order := make(chan int, 5)
		var workers sync.WaitGroup
		for i := range 5 {
			workers.Go(func() {
				p := acquire(t, &s)
				order <- i
				released := make(chan struct{})
				go func() {
					p.Release()
					close(released)
				}()
				<-released
			})
			synctest.Wait()
		}
		first.Release()
		workers.Wait()
		close(order)
		var got []int
		for i := range order {
			got = append(got, i)
		}
		if !reflect.DeepEqual(got, []int{0, 1, 2, 3, 4}) {
			t.Fatalf("admission order = %v", got)
		}
		p := s.TryAcquire()
		if p == nil {
			t.Fatal("released serial gate remained held")
		}
		p.Release()
	})
}

func TestKeyedSerialRetainsWaitersAndForgetsUnusedKeys(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var s gates.KeyedSerial[string]
		a := keyed(t, &s, "a")
		b := keyed(t, &s, "b")
		if s.Len() != 2 {
			t.Fatal("independent keys were not retained")
		}
		waiter := make(chan *gates.Permit, 1)
		go func() { waiter <- keyed(t, &s, "a") }()
		synctest.Wait()
		if len(waiter) != 0 {
			t.Fatal("same key admitted concurrently")
		}
		b.Release()
		if s.Len() != 1 {
			t.Fatal("unused key retained")
		}
		a.Release()
		again := <-waiter
		if s.Len() != 1 {
			t.Fatal("waiting key was removed")
		}
		again.Release()
		if s.Len() != 0 {
			t.Fatal("last release retained key")
		}
		// Cover both orders: the cancellation and the holder release each can
		// be the last reference. The Rust implementation leaked the latter case.
		for _, cancelFirst := range []bool{true, false} {
			held := keyed(t, &s, "c")
			ctx, cancel := context.WithCancel(t.Context())
			done := make(chan error, 1)
			go func() {
				p, err := s.Acquire(ctx, "c")
				if p != nil {
					p.Release()
				}
				done <- err
			}()
			synctest.Wait()
			cancel()
			if cancelFirst {
				if err := <-done; !errors.Is(err, context.Canceled) {
					t.Fatalf("waiter error = %v", err)
				}
				held.Release()
			} else {
				held.Release()
				if err := <-done; !errors.Is(err, context.Canceled) {
					t.Fatalf("waiter error = %v", err)
				}
			}
			if s.Len() != 0 {
				t.Fatal("canceled waiter retained key")
			}
		}
	})
}

func TestReleaseIsConcurrentAndIdempotent(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		a := gates.NewActivity(nil)
		l := enter(t, a, gates.Demand)
		var serial gates.Serial
		var keyedSerial gates.KeyedSerial[string]
		p := acquire(t, &serial)
		k := keyed(t, &keyedSerial, "key")
		var workers sync.WaitGroup
		for range 8 {
			workers.Go(func() {
				l.Release()
				p.Release()
				k.Release()
			})
		}
		workers.Wait()
		r := a.TryReserve()
		if r == nil {
			t.Fatal("lease retained permit")
		}
		for range 8 {
			workers.Go(r.Release)
		}
		workers.Wait()
		if s := a.State(); s.Reserved || s.Demand != 0 || keyedSerial.Len() != 0 {
			t.Fatal("repeated release retained ownership")
		}
		p = serial.TryAcquire()
		if p == nil {
			t.Fatal("serial permit retained")
		}
		defer p.Release()
		if extra := serial.TryAcquire(); extra != nil {
			extra.Release()
			t.Fatal("repeated release added extra permits")
		}
	})
}

func TestCanceledContextsAndSerialWait(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithCancel(t.Context())
		cancel()
		a := gates.NewActivity(nil)
		l, err := a.Enter(ctx, gates.Demand)
		if l != nil || !errors.Is(err, context.Canceled) {
			t.Fatalf("canceled enter = %v, %v", l, err)
		}
		r, err := a.Reserve(ctx)
		if r != nil || !errors.Is(err, context.Canceled) {
			t.Fatalf("canceled reserve = %v, %v", r, err)
		}
		var s gates.Serial
		held := acquire(t, &s)
		ctx, cancel = context.WithCancel(t.Context())
		defer cancel()
		done := make(chan error, 1)
		go func() {
			p, err := s.Acquire(ctx)
			if p != nil {
				p.Release()
			}
			done <- err
		}()
		synctest.Wait()
		cancel()
		if err := <-done; !errors.Is(err, context.Canceled) {
			t.Fatalf("serial waiter error = %v", err)
		}
		held.Release()
		p := s.TryAcquire()
		if p == nil {
			t.Fatal("canceled serial waiter retained permit")
		}
		p.Release()
	})
}
