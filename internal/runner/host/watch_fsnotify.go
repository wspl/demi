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
	addTree := func(root string) error {
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
	for _, tree := range trees {
		tree, err = filepath.EvalSymlinks(tree)
		if err == nil {
			tree, err = filepath.Abs(tree)
		}
		if err != nil {
			return nil, errors.Join(err, watcher.Close())
		}
		if err = addTree(tree); err != nil {
			return nil, errors.Join(err, watcher.Close())
		}
	}
	done := make(chan struct{})
	stop := make(chan struct{})
	go func() {
		defer close(done)
		for {
			select {
			case <-stop:
				return
			case event, ok := <-watcher.Events:
				if !ok {
					return
				}
				if event.Has(fsnotify.Create) {
					if info, err := os.Stat(event.Name); err == nil && info.IsDir() {
						// Files may have appeared before their new parent's subscription.
						// Invalidation covers that unavoidable recursive-watch installation gap.
						if err = addTree(event.Name); err != nil {
							report(WatchEvent{Kind: WatchFailed})
							return
						}
						report(WatchEvent{Kind: WatchLost})
					}
				}
				metadata := event.Op == fsnotify.Chmod
				report(WatchEvent{Kind: WatchChanged, Path: event.Name, Metadata: metadata})
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
	}()
	return func() {
		close(stop)
		_ = watcher.Close() // All notifications are discarded at explicit shutdown.
		<-done
	}, nil
}
