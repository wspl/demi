package tabs

import (
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
)

// Listed is a tab and the metadata last published by the registry.
type Listed struct {
	// Tab identifies the registered tab.
	Tab *Tab
	// Title holds the observed page title.
	Title string
	// URL holds the observed page URL.
	URL string
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
