package commandsdk_test

import (
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/commandsdk"
)

// The runner design retries a descriptor-starved call "at most a tenth of a
// second apart" until it succeeds; virtual time shows each gap without waiting.
func TestRetryWaitsOutDescriptorExhaustion(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var attempts []time.Time
		v, err := commandsdk.Retry(t.Context(), func() (int, error) {
			attempts = append(attempts, time.Now())
			if len(attempts) < 8 {
				return 0, commandsdk.Exhaustion()
			}
			return 42, nil
		})
		if err != nil || v != 42 {
			t.Fatalf("Retry = %d, %v; want 42, nil", v, err)
		}
		for i := 1; i < len(attempts); i++ {
			if gap := attempts[i].Sub(attempts[i-1]); gap <= 0 || gap > 100*time.Millisecond {
				t.Fatalf("gap before attempt %d = %s; want a wait of at most 100ms", i+1, gap)
			}
		}
	})
}
