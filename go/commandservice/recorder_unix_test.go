//go:build unix

package commandservice_test

import (
	"path/filepath"
	"syscall"
	"testing"
	"time"
)

// A path that is not a file, a pipe, is left alone: opening it can block, and
// recording it would wait for the job's lock too. With the lock held by another
// recording, the operation on the pipe still runs.
func TestAnOperationOnAPipeRunsWithoutTheJobsLock(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	pipe := filepath.Join(root, "pipe")
	if err := syscall.Mkfifo(pipe, 0o644); err != nil {
		t.Fatal(err)
	}
	holder := recorder.Begin()
	defer holder.Close()
	done := make(chan error, 1)
	go func() { done <- recorder.Record(pipe, func() error { return nil }) }()
	select {
	case err := <-done:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(hang):
		t.Fatal("the operation on a pipe waited for the job's lock")
	}
}
