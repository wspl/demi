package usershardtest_test

import (
	"context"
	"errors"
	"testing"
	"testing/synctest"

	"go.uber.org/goleak"

	"github.com/wspl/demi/internal/backend/usershard/usershardtest"
)

func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

// Channel-only fixture checks; synctest proves the held pass cannot complete.
func TestHoldCountsCancelledArrivalsAndReleasesWaiters(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var holds usershardtest.StepHolds[string]
		held := holds.Hold(t, "commit")
		ctx, cancel := context.WithCancel(t.Context())
		first := make(chan error, 1)
		go func() {
			first <- holds.Pass(ctx, "commit")
		}()
		if err := held.UntilArrived(t.Context(), 1); err != nil {
			t.Fatal(err)
		}
		cancel()
		if err := <-first; !errors.Is(err, context.Canceled) {
			t.Fatalf("cancelled pass = %v", err)
		}
		second := make(chan error, 1)
		go func() {
			second <- holds.Pass(t.Context(), "commit")
		}()
		if err := held.UntilArrived(t.Context(), 2); err != nil {
			t.Fatal(err)
		}
		synctest.Wait()
		select {
		case <-second:
			t.Fatal("pass escaped its hold")
		default:
		}
		if err := holds.Pass(t.Context(), "different"); err != nil {
			t.Fatal(err)
		}
		held.Release()
		held.Release()
		if err := <-second; err != nil {
			t.Fatal(err)
		}
		if err := holds.Pass(t.Context(), "commit"); err != nil {
			t.Fatal(err)
		}
	})
}
