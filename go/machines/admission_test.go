package machines

import (
	"sync"
	"testing"
	"testing/synctest"
)

func TestExclusiveEntryWaitsForSharedWorkAndHoldsBackLaterEntrants(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := newAdmission()
		running, err := gate.enter(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		var mu sync.Mutex
		var order []string
		record := func(step string) {
			mu.Lock()
			defer mu.Unlock()
			order = append(order, step)
		}
		go func() {
			whole, err := gate.exclusive(t.Context())
			if err != nil {
				t.Error(err)
				return
			}
			record("exclusive")
			record("exclusive done")
			whole.Release()
		}()
		synctest.Wait()
		go func() {
			later, err := gate.enter(t.Context())
			if err != nil {
				t.Error(err)
				return
			}
			record("later")
			later.Release()
		}()
		synctest.Wait()
		mu.Lock()
		waiting := len(order)
		mu.Unlock()
		if waiting != 0 {
			t.Fatalf("something ran while shared work was in flight: %v", order)
		}
		running.Release()
		synctest.Wait()
		mu.Lock()
		defer mu.Unlock()
		if want := []string{"exclusive", "exclusive done", "later"}; len(order) != 3 || order[0] != want[0] || order[1] != want[1] || order[2] != want[2] {
			t.Errorf("order %v, want %v", order, want)
		}
	})
}

func TestAClosedGateRefusesWaitingAndNewEntrants(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		gate := newAdmission()
		whole, err := gate.exclusive(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		waiting := make(chan error, 1)
		go func() {
			lease, err := gate.enter(t.Context())
			if err == nil {
				lease.Release()
			}
			waiting <- err
		}()
		synctest.Wait()
		gate.close()
		whole.Release()
		if err := <-waiting; err != ErrStopping {
			t.Errorf("a waiting entrant: %v", err)
		}
		if _, err := gate.enter(t.Context()); err != ErrStopping {
			t.Errorf("a new entrant: %v", err)
		}
		if _, err := gate.exclusive(t.Context()); err != ErrStopping {
			t.Errorf("a new exclusive entrant: %v", err)
		}
	})
}
