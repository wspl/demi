package process_test

import (
	"context"
	"errors"
	"syscall"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/internal/runner/process"

	"github.com/wspl/demi/internal/cmdsdk"
)

func TestStartRetriesOnlyTransientFailures(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		calls := 0
		value, err := process.Start(t.Context(), func() (int, error) {
			calls++
			if calls < 3 {
				return 0, cmdsdk.Exhaustion()
			}
			return 42, nil
		})
		if err != nil || value != 42 || calls != 3 {
			t.Fatalf("descriptor retry: %d %d %v", value, calls, err)
		}
		start := time.Now()
		_, err = process.Start(t.Context(), func() (int, error) { return 0, syscall.ETXTBSY })
		if !errors.Is(err, syscall.ETXTBSY) || time.Since(start) != 1055*time.Millisecond {
			t.Fatalf("busy retry: %s %v", time.Since(start), err)
		}
		calls = 0
		_, err = process.Start(t.Context(), func() (int, error) {
			calls++
			return 0, syscall.EACCES
		})
		if !errors.Is(err, syscall.EACCES) || calls != 1 {
			t.Fatalf("permanent failure retried: %d %v", calls, err)
		}
		ctx, cancel := context.WithCancel(t.Context())
		done := make(chan error, 1)
		go func() {
			_, err := process.Start(ctx, func() (int, error) { return 0, cmdsdk.Exhaustion() })
			done <- err
		}()
		synctest.Wait()
		cancel()
		if err := <-done; !errors.Is(err, context.Canceled) {
			t.Fatalf("cancel retry: %v", err)
		}
	})
}
