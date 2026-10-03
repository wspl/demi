//go:build linux || windows

package host

import (
	"context"
	"errors"
	"io/fs"
	"os"
	"path/filepath"

	"github.com/fsnotify/fsnotify"
)

// startPlatformWatch installs recursive directory subscriptions and owns their event loop.
func startPlatformWatch(ctx context.Context, trees []string, report func(WatchEvent)) (func(), error) {
	watcher, err := fsnotify.NewWatcher()
	if err != nil {
		return nil, err
	}
	for _, tree := range trees {
		tree, err = filepath.EvalSymlinks(tree)
		if err == nil {
			tree, err = filepath.Abs(tree)
		}
		if err != nil {
			return nil, errors.Join(err, watcher.Close())
		}
		if err = subscribeTree(ctx, watcher, tree); err != nil {
			return nil, errors.Join(err, watcher.Close())
		}
	}
	done := make(chan struct{})
	stop := make(chan struct{})
	go watchEvents(ctx, watcher, report, done, stop)
	return func() {
		close(stop)
		_ = watcher.Close() // All notifications are discarded at explicit shutdown.
		<-done
	}, nil
}

func subscribeTree(ctx context.Context, watcher *fsnotify.Watcher, root string) error {
	return filepath.WalkDir(root, func(path string, entry fs.DirEntry, failure error) error {
		if err := ctx.Err(); err != nil {
			return err
		}
		if failure != nil {
			return failure
		}
		if entry.IsDir() {
			return watcher.Add(path)
		}
		return nil
	})
}

func watchEvents(
	ctx context.Context,
	watcher *fsnotify.Watcher,
	report func(WatchEvent),
	done chan<- struct{},
	stop <-chan struct{},
) {
	defer close(done)
	for {
		select {
		case <-stop:
			return
		case event, ok := <-watcher.Events:
			if !ok {
				return
			}
			if !reportWatchEvent(ctx, watcher, event, report) {
				return
			}
		case failure, ok := <-watcher.Errors:
			if !ok {
				return
			}
			if errors.Is(failure, fsnotify.ErrEventOverflow) {
				report(WatchEvent{Kind: WatchLost})
			} else {
				report(WatchEvent{Kind: WatchFailed})
				return
			}
		}
	}
}

// reportWatchEvent invalidates the subscription gap before reporting the triggering path.
func reportWatchEvent(
	ctx context.Context,
	watcher *fsnotify.Watcher,
	event fsnotify.Event,
	report func(WatchEvent),
) bool {
	if event.Has(fsnotify.Create) {
		if info, err := os.Stat(event.Name); err == nil && info.IsDir() {
			// Files may have appeared before their new parent's subscription.
			// Invalidation covers that unavoidable recursive-watch installation gap.
			if err = subscribeTree(ctx, watcher, event.Name); err != nil {
				report(WatchEvent{Kind: WatchFailed})
				return false
			}
			report(WatchEvent{Kind: WatchLost})
		}
	}
	metadata := event.Op == fsnotify.Chmod
	report(WatchEvent{Kind: WatchChanged, Path: event.Name, Metadata: metadata})
	return true
}
