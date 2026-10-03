package host

import (
	"context"
	"os"
	"path/filepath"
	"sync"
	"testing"
)

func TestNativeWatchRecursiveDeliveryAndClose(t *testing.T) {
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	events := make(chan WatchEvent, 1024)
	var mu sync.Mutex
	reported := 0
	watch, err := StartWatch(
		t.Context(),
		[]string{root},
		func(e WatchEvent) {
			mu.Lock()
			reported++
			mu.Unlock()
			events <- e
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := watch.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	// A new subtree must be covered, including files created before recursive
	// Linux subscriptions can be installed; WatchLost covers that startup gap.
	writeFixture(t, root, "new/deep/file", "first\n")
	for {
		event := <-events
		if event.Kind == WatchLost || event.Path == filepath.Join(root, "new/deep/file") {
			break
		}
	}
	writeFixture(t, root, "new/deep/file", "second\n")
	for {
		event := <-events
		if event.Kind == WatchChanged && event.Path == filepath.Join(root, "new/deep/file") {
			break
		}
	}
	if err := os.Chmod(filepath.Join(root, "new/deep/file"), 0o755); err != nil {
		t.Fatal(err)
	}
	for {
		event := <-events
		if event.Kind == WatchChanged && event.Path == filepath.Join(root, "new/deep/file") {
			break
		}
	}
	if err := watch.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
	mu.Lock()
	atClose := reported
	mu.Unlock()
	writeFixture(t, root, "after-close", "no callback\n")
	if err := watch.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
	mu.Lock()
	defer mu.Unlock()
	if reported != atClose {
		t.Fatal("callback after Close")
	}
}

func TestWatchStartupFailureAndCancellation(t *testing.T) {
	if watch, err := StartWatch(
		t.Context(),
		[]string{filepath.Join(t.TempDir(), "missing")},
		func(WatchEvent) {},
	); err == nil ||
		watch != nil {
		t.Fatalf("missing root: %v %v", watch, err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if watch, err := StartWatch(ctx, []string{t.TempDir()}, func(WatchEvent) {}); err == nil || watch != nil {
		t.Fatalf("canceled: %v %v", watch, err)
	}
}
