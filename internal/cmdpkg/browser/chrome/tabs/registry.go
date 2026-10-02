package tabs

import (
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// Listed is a tab and the metadata last published by the registry.
type Listed struct {
	Tab   *Tab
	Title string
	URL   string
}

// Snapshot is an immutable publication; callers must not mutate its slice.
type Snapshot struct {
	// Tabs are the live tabs in Chrome creation order.
	Tabs []Listed
	// Registering reports pending setup or event-loss reconciliation.
	Registering bool
	// Changed closes when a newer snapshot is published.
	Changed <-chan struct{}
}

// Find returns the tab with this public ID, or nil if absent.
func (s *Snapshot) Find(id browserop.TabID) *Tab {
	for _, listed := range s.Tabs {
		if listed.Tab.ID() == id {
			return listed.Tab
		}
	}
	return nil
}

// Closed describes whether closing a tab empties its environment.
type Closed uint8

const (
	// ClosedTab means the tab is gone from Chrome and the registry.
	ClosedTab Closed = iota + 1
	// ClosedEnvironment means the final tab sealed the registry for retirement.
	ClosedEnvironment
)
