package accounts

import (
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/webapi"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

func TestLimiterForgetsAddressAfterLockWindow(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		limiter := NewLoginLimiter()
		for _, address := range []webapi.EmailAddress{"a@example.test", "b@example.test", "c@example.test"} {
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

func TestEachFailureExtendsWindowAndSuccessClearsIt(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		limiter := NewLoginLimiter()
		email := webapi.EmailAddress("ana@example.test")
		for range 4 {
			limiter.Failed(email)
			time.Sleep(59 * time.Second)
		}
		limiter.Failed(email)
		if !limiter.Locked(email) {
			t.Fatal("fifth failure did not lock")
		}
		time.Sleep(59 * time.Second)
		if !limiter.Locked(email) {
			t.Fatal("lock expired early")
		}
		time.Sleep(time.Second)
		if limiter.Locked(email) {
			t.Fatal("lock did not expire")
		}
		for range 4 {
			limiter.Failed(email)
		}
		limiter.Succeeded(email)
		limiter.Failed(email)
		if limiter.Locked(email) {
			t.Fatal("success did not clear failures")
		}
	})
}
