package providers_test

import (
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/backend/providers"
)

func TestRequestsAdmittedAsEarliestLeaveWindow(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		limit := providers.NewRequestRateLimit(3)
		for range 2 {
			if err := limit.Take(); err != nil {
				t.Fatal(err)
			}
		}
		time.Sleep(30 * time.Second)
		if err := limit.Take(); err != nil {
			t.Fatal(err)
		}
		if err := limit.Take(); err == nil || err.Error() != "Provider request rate limit reached (3 per minute)" {
			t.Fatal(err)
		}
		time.Sleep(30 * time.Second)
		for range 2 {
			if err := limit.Take(); err != nil {
				t.Fatal(err)
			}
		}
		if err := limit.Take(); err == nil || err.Error() != "Provider request rate limit reached (3 per minute)" {
			t.Fatal(err)
		}
	})
}
