package host

import (
	"context"
	"os"
	"path/filepath"
	"testing"
)

func TestFSEventsIgnoresReadsAndReleasesNativeStream(t *testing.T) {
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	before := activeStreams.Load()
	events := make(chan WatchEvent, 1024)
	watch, err := StartWatch(t.Context(), []string{root}, func(event WatchEvent) {
		events <- event
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := watch.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	if got := activeStreams.Load(); got != before+1 {
		t.Fatalf("streams: %d -> %d", before, got)
	}
	writeFixture(t, root, "read-only", "content\n")
	for {
		event := <-events
		if event.Path == filepath.Join(root, "read-only") {
			break
		}
	}
	// The first notification can precede the initial write's remaining
	// notifications. A later file event separates that write from the read.
	writeFixture(t, root, "write-boundary", "event\n")
	for {
		event := <-events
		if event.Path == filepath.Join(root, "write-boundary") {
			break
		}
	}
	if _, err := os.ReadFile(filepath.Join(root, "read-only")); err != nil {
		t.Fatal(err)
	}
	writeFixture(t, root, "barrier", "event\n")
	for {
		event := <-events
		if event.Kind == WatchChanged && event.Path == filepath.Join(root, "read-only") {
			t.Fatalf("read reported as change: %+v", event)
		}
		if event.Path == filepath.Join(root, "barrier") {
			break
		}
	}
	if err := watch.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
	if got := activeStreams.Load(); got != before {
		t.Fatalf("stream leak: %d -> %d", before, got)
	}
}
