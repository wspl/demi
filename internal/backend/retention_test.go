package backend_test

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/fsnotify/fsnotify"
	"github.com/wspl/demi/internal/agent/store/storetest"
	"github.com/wspl/demi/internal/backend/blobs/blobstest"
)

// Restart over local files, then wait on the deletion event, never elapsed time.
func TestRetentionRunsFirstPassAfterServing(t *testing.T) {
	ctx, h := filesHarness(t)
	counts := &blobstest.ObjectCounts{}
	h.Objects = counts
	h.Clock.FollowSystem()
	interval := 24 * time.Hour
	h.Config.Lifecycle.RetentionInterval = &interval
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	wireMust(t, b.Close(ctx))
	name := filesBlob(t, h, string(s.User.ID), storetest.PNG(2, 2, 4))
	path := filepath.Join(h.DataDir(), "blobs", string(s.User.ID), name)
	now, err := time.Parse(time.RFC3339Nano, string(h.Clock.Now()))
	wireMust(t, err)
	written := now.Add(-25 * time.Hour)
	wireMust(t, os.Chtimes(path, written, written))
	watcher, err := fsnotify.NewWatcher()
	wireMust(t, err)
	defer func() { wireMust(t, watcher.Close()) }()
	wireMust(t, watcher.Add(filepath.Dir(path)))
	b, err = h.Start(ctx, t)
	wireMust(t, err)
	for {
		_, err := os.Stat(path)
		if errors.Is(err, os.ErrNotExist) {
			break
		}
		wireMust(t, err)
		select {
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		case err := <-watcher.Errors:
			wireMust(t, err)
		case <-watcher.Events:
		}
	}
	if got := counts.Tally().Deletes; got != 1 {
		t.Fatalf("deletes = %d, want 1", got)
	}
	wireMust(t, b.Close(ctx))
}
