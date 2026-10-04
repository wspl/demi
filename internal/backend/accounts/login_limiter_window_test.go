package accounts_test

import (
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/backend/accounts"
	"github.com/wspl/demi/internal/webapiproto"
)

func TestEachFailureExtendsWindowAndSuccessClearsIt(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		limiter := accounts.NewLoginLimiter()
		email := webapiproto.EmailAddress("ana@example.test")
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
