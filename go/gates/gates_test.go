package gates

import (
	"context"
	"reflect"
	"testing"
	"testing/synctest"
	"time"
)

// Every test runs in a synctest bubble: goroutines that are blocked on a gate
// are known to be blocked, so a test waits for them with synctest.Wait, never
// for time.

// events collects what the goroutines of a test did, in order.
type events chan string

func (e events) drain() []string {
	var got []string
	for {
		select {
		case event := <-e:
			got = append(got, event)
		default:
			return got
		}
	}
}

func TestAReservationDrainsCurrentWorkAndQueuesNewWorkUntilItEnds(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx := context.Background()
		gate := NewActivityGate()
		first, _ := gate.Enter(ctx, Demand)
		second, _ := gate.Enter(ctx, Demand)
		if _, ok := gate.TryReserve(); ok {
			t.Fatal("the gate is reserved while two leases hold it")
		}
		got := make(events, 4)
		reserved := make(chan *Reservation, 1)
		go func() {
			reservation, err := gate.Reserve(ctx)
			if err != nil {
				t.Error(err)
			}
			got <- "writer"
			reserved <- reservation
		}()
		synctest.Wait()
		// A waiting reservation holds back later entrants, and TryEnter fails.
		if _, ok := gate.TryEnter(Demand); ok {
			t.Fatal("a lease was admitted ahead of a waiting reservation")
		}
		entered := make(chan *Lease, 1)
		go func() {
			lease, err := gate.Enter(ctx, Demand)
			if err != nil {
				t.Error(err)
			}
			got <- "entrant"
			entered <- lease
		}()
		synctest.Wait()
		first.Release()
		synctest.Wait()
		if events := got.drain(); len(events) != 0 {
			t.Fatalf("events = %v after one of two leases ended", events)
		}
		second.Release()
		synctest.Wait()
		if events := got.drain(); !reflect.DeepEqual(events, []string{"writer"}) {
			t.Fatalf("events = %v, want the reservation alone", events)
		}
		if !gate.State().Reserved {
			t.Error("the state does not show the reservation")
		}
		(<-reserved).Release()
		synctest.Wait()
		if events := got.drain(); !reflect.DeepEqual(events, []string{"entrant"}) {
			t.Fatalf("events = %v, want the entrant after the reservation", events)
		}
		lease := <-entered
		if state := gate.State(); state.Demand != 1 || state.Reserved {
			t.Errorf("state = %+v, want one demand lease and no reservation", state)
		}
		lease.Release()
		if state := gate.State(); state.Demand != 0 {
			t.Errorf("demand = %d after the last lease ended", state.Demand)
		}
	})
}

func TestGivingUpADrainingReservationAdmitsWaitersWithoutTouchingLeases(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := NewActivityGate()
		held, _ := gate.Enter(context.Background(), Demand)
		writerContext, giveUp := context.WithCancel(context.Background())
		writer := make(chan error, 1)
		go func() {
			_, err := gate.Reserve(writerContext)
			writer <- err
		}()
		synctest.Wait()
		entered := make(chan *Lease, 1)
		go func() {
			lease, _ := gate.Enter(context.Background(), Demand)
			entered <- lease
		}()
		synctest.Wait()
		if len(entered) != 0 {
			t.Fatal("an entrant was admitted ahead of a waiting reservation")
		}
		giveUp()
		if err := <-writer; err != context.Canceled {
			t.Fatalf("the reservation's wait ended with %v, want the cancellation", err)
		}
		lease := <-entered
		if state := gate.State(); state.Demand != 2 || state.Reserved {
			t.Errorf("state = %+v, want the two leases and no reservation", state)
		}
		lease.Release()
		held.Release()
		reservation, ok := gate.TryReserve()
		if !ok {
			t.Fatal("an idle gate does not reserve")
		}
		reservation.Release()
	})
}

func TestAnIdleReservationExcludesEntrantsAndAGivenUpWaitNeverAdmits(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := NewActivityGate()
		reservation, ok := gate.TryReserve()
		if !ok {
			t.Fatal("an idle gate does not reserve")
		}
		if _, ok := gate.TryReserve(); ok {
			t.Error("a reserved gate is reserved again")
		}
		if _, ok := gate.TryEnter(Maintenance); ok {
			t.Error("a lease was admitted to a reserved gate")
		}
		waitContext, giveUp := context.WithCancel(context.Background())
		waiting := make(chan error, 1)
		go func() {
			_, err := gate.Enter(waitContext, Demand)
			waiting <- err
		}()
		synctest.Wait()
		giveUp()
		if err := <-waiting; err != context.Canceled {
			t.Fatalf("the wait ended with %v, want the cancellation", err)
		}
		reservation.Release()
		if state := gate.State(); state.Demand != 0 || state.Reserved {
			t.Errorf("state = %+v after the reservation, want it empty", state)
		}
		if _, ok := gate.TryReserve(); !ok {
			t.Error("the gate is not free after a given-up wait")
		}
	})
}

func TestAWaitThatIsAlreadyCancelledHoldsNothing(t *testing.T) {
	gate := NewActivityGate()
	cancelled, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := gate.Enter(cancelled, Demand); err != context.Canceled {
		t.Errorf("Enter: err = %v", err)
	}
	if _, err := gate.Reserve(cancelled); err != context.Canceled {
		t.Errorf("Reserve: err = %v", err)
	}
	if _, err := NewSerialGate().Acquire(cancelled); err != context.Canceled {
		t.Errorf("Acquire: err = %v", err)
	}
	if state := gate.State(); state != (GateState{}) {
		t.Errorf("state = %+v, want none held", state)
	}
}

func TestTheStateCountsLeasesByPurposeAndRecordsWhenDemandEnds(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var hub ActivityHub
		gate := hub.NewGate()
		// changed reports whether the gate and the hub were signalled by f.
		changed := func(f func()) bool {
			gateChanged, hubChanged := gate.Changed(), hub.Changed()
			f()
			woken := func(c <-chan struct{}) bool {
				select {
				case <-c:
					return true
				default:
					return false
				}
			}
			return woken(gateChanged) && woken(hubChanged)
		}
		var maintenance, demand *Lease
		if !changed(func() { maintenance, _ = gate.TryEnter(Maintenance) }) {
			t.Error("a lease entering did not signal")
		}
		if !changed(func() { demand, _ = gate.Enter(context.Background(), Demand) }) {
			t.Error("a lease entering did not signal")
		}
		if state := gate.State(); state.Demand != 1 || state.Maintenance != 1 || !state.LastDemandEnd.IsZero() {
			t.Fatalf("state = %+v, want one of each and no demand ended", state)
		}
		time.Sleep(5 * time.Second)
		ended := time.Now()
		if !changed(demand.Release) {
			t.Error("a lease ending did not signal")
		}
		if got := gate.State().LastDemandEnd; !got.Equal(ended) {
			t.Fatalf("last demand end = %v, want %v", got, ended)
		}
		// Maintenance ending is not demand ending.
		time.Sleep(5 * time.Second)
		if !changed(maintenance.Release) {
			t.Error("a lease ending did not signal")
		}
		if state := gate.State(); !state.LastDemandEnd.Equal(ended) || state.Maintenance != 0 {
			t.Errorf("state = %+v, want the same demand end and no maintenance", state)
		}
		// Releasing again changes nothing.
		if changed(demand.Release) {
			t.Error("a lease released twice signalled twice")
		}
		if state := gate.State(); state.Demand != 0 {
			t.Errorf("demand = %d after a double release", state.Demand)
		}
	})
}

func TestASerialGateAdmitsInArrivalOrderAndItsPermitMovesIntoAGoroutine(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx := context.Background()
		gate := NewSerialGate()
		first, _ := gate.Acquire(ctx)
		if _, ok := gate.TryAcquire(); ok {
			t.Fatal("a held gate admitted a second holder")
		}
		order := make(chan int, 5)
		done := make(chan struct{}, 5)
		for i := range 5 {
			go func() {
				permit, _ := gate.Acquire(ctx)
				order <- i
				// The permit travels into the work it admits.
				go func() {
					time.Sleep(time.Second)
					permit.Release()
					done <- struct{}{}
				}()
			}()
			synctest.Wait()
		}
		first.Release()
		for range 5 {
			<-done
		}
		var got []int
		for range 5 {
			got = append(got, <-order)
		}
		if want := []int{0, 1, 2, 3, 4}; !reflect.DeepEqual(got, want) {
			t.Errorf("order = %v, want %v", got, want)
		}
		if _, ok := gate.TryAcquire(); !ok {
			t.Error("the gate is not free after every holder released")
		}
	})
}

func TestKeyedGatesSerializeOneKeyAndForgetItOnceNobodyUsesIt(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx := context.Background()
		gates := NewKeyedSerialGate[string]()
		a, _ := gates.Acquire(ctx, "a")
		// Another key does not wait.
		b, _ := gates.Acquire(ctx, "b")
		if gates.Len() != 2 {
			t.Fatalf("keys = %d, want 2", gates.Len())
		}
		waiter := make(chan *KeyedPermit[string], 1)
		go func() {
			permit, _ := gates.Acquire(ctx, "a")
			waiter <- permit
		}()
		synctest.Wait()
		if len(waiter) != 0 {
			t.Fatal("a second holder of one key was admitted")
		}
		b.Release()
		if gates.Len() != 1 {
			t.Errorf("keys = %d after b was released, want only a", gates.Len())
		}
		a.Release()
		again := <-waiter
		if gates.Len() != 1 {
			t.Errorf("keys = %d, want a while its waiter holds it", gates.Len())
		}
		again.Release()
		if gates.Len() != 0 {
			t.Errorf("keys = %d, want none in use", gates.Len())
		}
		// A waiter that gives up leaves nothing behind once the holder leaves.
		held, _ := gates.Acquire(ctx, "c")
		quitContext, giveUp := context.WithCancel(ctx)
		quit := make(chan error, 1)
		go func() {
			_, err := gates.Acquire(quitContext, "c")
			quit <- err
		}()
		synctest.Wait()
		giveUp()
		if err := <-quit; err != context.Canceled {
			t.Fatalf("the wait ended with %v, want the cancellation", err)
		}
		held.Release()
		if gates.Len() != 0 {
			t.Errorf("keys = %d after the holder left, want none", gates.Len())
		}
	})
}
