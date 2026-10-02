package host

//revive:disable:unused-parameter // API checkpoint: parameter names document the boundary.

import "context"

// WatchEventKind identifies a changed path, lost events, or a failed watch.
type WatchEventKind uint8

const (
	// WatchChanged reports a path whose content or metadata changed.
	WatchChanged WatchEventKind = iota
	// WatchLost invalidates the baseline because events were lost.
	WatchLost
	// WatchFailed ends incremental observation; future requests walk the whole tree.
	WatchFailed
)

// WatchEvent is a filesystem notification, not a wire contract.
type WatchEvent struct {
	// Kind determines whether Path and Metadata describe a change.
	Kind WatchEventKind
	// Path is the changed absolute path for WatchChanged.
	Path string
	// Metadata means only mode, ownership or timestamps changed.
	Metadata bool
}

// Watch owns recursive filesystem subscriptions and their notification work.
// Its owner cancels the lifetime context and calls Close to release subscriptions.
type Watch struct{}

// StartWatch watches every tree recursively with FSEvents on macOS and fsnotify
// on Linux and Windows. Setup may block; GitChanges starts it outside the request
// path. A setup failure returns an error and no watch. Notifications ignore reads
// and invoke report serially. The callback must return promptly and must not call
// Close. Lost notifications request a whole scan, never a silently partial view.
func StartWatch(ctx context.Context, trees []string, report func(WatchEvent)) (*Watch, error) {
	panic("not written: r-host")
}

// Close stops subscriptions and joins callbacks before returning. It is
// idempotent; use a live cleanup context after canceling the watch's lifetime.
// Another Close can resume a wait interrupted by cleanup-context cancellation.
func (w *Watch) Close(ctx context.Context) error {
	panic("not written: r-host")
}
