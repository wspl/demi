package process_test

import (
	"context"
	"errors"
	"fmt"
	"syscall"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/runner/process"
)

// Retry scenarios use fake time and allocate no processes.
func TestStartRetriesTransientFailures(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		started := time.Now()
		attempts := 0
		value, err := process.Start(t.Context(), func() (int, error) {
			attempts++
			if attempts <= 20 {
				return 0, fmt.Errorf("start: %w", exhaustedError())
			}
			if attempts == 21 {
				return 0, fmt.Errorf("start: %w", syscall.ETXTBSY)
			}
			return 42, nil
		})
		if err != nil || value != 42 || attempts != 22 {
			t.Fatalf("value=%d attempts=%d err=%v", value, attempts, err)
		}
		if elapsed := time.Since(started); elapsed != 1755*time.Millisecond {
			t.Fatalf("retry spacing: %v", elapsed)
		}
	})
}

func TestBusyBudgetAndPermanentErrors(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		started := time.Now()
		attempts := 0
		_, err := process.Start(t.Context(), func() (int, error) {
			attempts++
			return 0, fmt.Errorf("start: %w", syscall.ETXTBSY)
		})
		if !errors.Is(err, syscall.ETXTBSY) || time.Since(started) != time.Second || attempts != 15 {
			t.Fatalf("attempts=%d elapsed=%v err=%v", attempts, time.Since(started), err)
		}
		attempts = 0
		_, err = process.Start(t.Context(), func() (int, error) {
			attempts++
			return 0, syscall.EACCES
		})
		if !errors.Is(err, syscall.EACCES) || attempts != 1 {
			t.Fatalf("attempts=%d err=%v", attempts, err)
		}
	})
}

func TestCancellationInterruptsStartWait(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		done := make(chan error, 1)
		attempts := 0
		go func() {
			_, err := process.Start(ctx, func() (int, error) {
				attempts++
				return 0, exhaustedError()
			})
			done <- err
		}()
		synctest.Wait()
		cancel()
		if err := <-done; !errors.Is(err, context.Canceled) || attempts != 1 {
			t.Fatalf("attempts=%d err=%v", attempts, err)
		}
	})
}
