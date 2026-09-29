package shell

import (
	"os"
	"path/filepath"
	"testing"
	"testing/synctest"

	"golang.org/x/sys/unix"
)

// A busy probe executable becomes runnable after its first failed start. Fake
// time waits for the retry timer before releasing the writer; no timed sleep.
func TestInheritedLimitProbeRetriesFreshCommand(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		path := filepath.Join(t.TempDir(), "probe")
		if err := os.WriteFile(path, []byte("#!/bin/sh\nprintf '128\\n256\\n'\n"), 0700); err != nil {
			t.Fatal(err)
		}
		writer, err := os.OpenFile(path, os.O_WRONLY, 0)
		if err != nil {
			t.Fatal(err)
		}
		defer writer.Close()
		type result struct {
			limit unix.Rlimit
			err   error
		}
		done := make(chan result, 1)
		go func() {
			limit, err := probeOpenFiles(t.Context(), path)
			done <- result{limit, err}
		}()
		synctest.Wait()
		if err := writer.Close(); err != nil {
			t.Fatal(err)
		}
		got := <-done
		if got.err != nil || got.limit.Cur != 128 || got.limit.Max != 256 {
			t.Fatalf("retried probe: %+v, %v", got.limit, got.err)
		}
	})
}
