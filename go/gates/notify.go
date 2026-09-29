package gates

import "sync"

// A notifier wakes whoever waits for the next change of something. Nothing is
// allocated while nobody waits.
type notifier struct {
	mu      sync.Mutex
	changed chan struct{}
}

// next returns a channel that is closed at the next change. Read the state
// after taking the channel and before waiting on it, and no change is lost.
func (n *notifier) next() <-chan struct{} {
	n.mu.Lock()
	defer n.mu.Unlock()
	if n.changed == nil {
		n.changed = make(chan struct{})
	}
	return n.changed
}

// notify wakes everyone who took a channel since the last change.
func (n *notifier) notify() {
	n.mu.Lock()
	defer n.mu.Unlock()
	if n.changed != nil {
		close(n.changed)
		n.changed = nil
	}
}
