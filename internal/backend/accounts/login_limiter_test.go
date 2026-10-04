package accounts

import (
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/webapiproto"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) {
	goleak.VerifyTestMain(m)
}

func TestLimiterForgetsAddressAfterLockWindow(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		limiter := NewLoginLimiter()
		for _, address := range []webapiproto.EmailAddress{"a@example.test", "b@example.test", "c@example.test"} {
			limiter.Failed(address)
		}
		for range 4 {
			limiter.Failed("a@example.test")
		}
		if !limiter.Locked("a@example.test") || limiter.Locked("b@example.test") {
			t.Fatal("only a must be locked")
		}
		if len(limiter.failures) != 3 {
			t.Fatal("expected three tracked addresses")
		}
		time.Sleep(time.Minute) // synctest advances the lock's exact deadline.
		if limiter.Locked("a@example.test") {
			t.Fatal("lock did not expire")
		}
		limiter.Failed("d@example.test")
		if len(limiter.failures) != 1 {
			t.Fatal("expired sprayed addresses were not swept")
		}
	})
}
