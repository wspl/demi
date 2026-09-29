//go:build unix

package interp

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"sync/atomic"
	"testing"
	"testing/synctest"

	"golang.org/x/sys/unix"
)

// Cancellation of a descriptor-starved opener needs no new descriptor. Fake
// time orders the two workers; no process or timed sleep is used.
func TestCanceledFIFOOpenerNeedsNoCleanupDescriptor(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		path := filepath.Join(t.TempDir(), "fifo")
		if err := unix.Mkfifo(path, 0600); err != nil {
			t.Fatal(err)
		}
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		entered, release := make(chan struct{}), make(chan struct{})
		var calls atomic.Int64
		handler := RetryHandlerFunc(func(ctx context.Context, attempt func() error) error {
			if calls.Add(1) == 1 {
				close(entered)
				<-release
				return ctx.Err()
			}
			close(release)
			// The first opener must publish its canceled result before cleanup tries
			// to allocate. Removing the FIFO makes an unnecessary open observable.
			synctest.Wait()
			if err := os.Remove(path); err != nil {
				return err
			}
			return attempt()
		})
		ctx = context.WithValue(ctx, retryKey{}, handler)
		result := make(chan error, 1)
		go func() {
			file, err := openContext(ctx, path, os.O_RDONLY, 0)
			if file != nil {
				file.Close()
			}
			result <- err
		}()
		<-entered
		cancel()
		if err := <-result; !errors.Is(err, context.Canceled) {
			t.Fatalf("canceled opener: %v", err)
		}
	})
}
