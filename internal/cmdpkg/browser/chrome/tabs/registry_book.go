package tabs

import (
	"slices"
	"strconv"

	"github.com/chromedp/cdproto/target"
	"github.com/wspl/demi/internal/cmdpkg/browser/browserop"
	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/cdp"
)

type tabStage uint8

const (
	tabPending tabStage = iota
	tabSetup
	tabLive
	tabClosing
	tabUnusable
)

type registryEntry struct {
	title, url string
	stage      tabStage
	tab        *Tab
}
type publicTab struct {
	id    browserop.TabID
	order int
}

// registryBook is the registry owner's tab state; it performs no IO.
type registryBook struct {
	entries     map[target.ID]*registryEntry
	publicIDs   map[target.ID]publicTab
	openers     map[target.ID]target.ID
	destroyed   []target.ID
	creating    int
	sealed      bool
	reconciling bool
}

func newRegistryBook() *registryBook {
	return &registryBook{entries: make(map[target.ID]*registryEntry), publicIDs: make(map[target.ID]publicTab), openers: make(map[target.ID]target.ID)}
}

// found remembers first-sighting openers even when later Chrome metadata omits them.
func (b *registryBook) found(info *target.Info) bool {
	if info.Type != "page" || slices.Contains(b.destroyed, info.TargetID) {
		return false
	}
	if info.OpenerID != "" {
		if _, ok := b.openers[info.TargetID]; !ok {
			b.openers[info.TargetID] = info.OpenerID
		}
	}
	entry := b.entries[info.TargetID]
	if entry == nil {
		entry = &registryEntry{}
		b.entries[info.TargetID] = entry
	}
	changed := entry.title != info.Title || entry.url != info.URL
	entry.title = info.Title
	entry.url = info.URL
	return changed && (entry.stage == tabLive || entry.stage == tabClosing)
}

// retitle applies title refreshes only while the tab still shows the same URL.
func (b *registryBook) retitle(info *target.Info) bool {
	entry := b.entries[info.TargetID]
	if entry == nil || entry.url != info.URL || entry.title == info.Title {
		return false
	}
	entry.title = info.Title
	return entry.stage == tabLive || entry.stage == tabClosing
}

func (b *registryBook) admit() bool {
	if b.sealed {
		return false
	}
	b.creating++
	return true
}
func (b *registryBook) failed() { b.creating-- }
func (b *registryBook) pending(id target.ID) bool {
	entry := b.entries[id]
	return entry != nil && entry.stage == tabPending
}
func (b *registryBook) setUp(id target.ID) {
	entry := b.entries[id]
	if entry == nil {
		entry = &registryEntry{}
		b.entries[id] = entry
	}
	entry.stage = tabSetup
}
func (b *registryBook) unusable(id target.ID) {
	if entry := b.entries[id]; entry != nil {
		entry.stage = tabUnusable
	}
}

// ready refuses a completed setup whose page disappeared or environment sealed.
func (b *registryBook) ready(id target.ID, created bool, tab *Tab) error {
	if created {
		b.creating--
	}
	var refused error
	if b.sealed {
		refused = &cdp.BrowserError{Kind: cdp.KindClosed}
	} else if slices.Contains(b.destroyed, id) {
		refused = &cdp.BrowserError{Kind: cdp.KindTabNotFound}
	}
	if entry := b.entries[id]; entry != nil {
		entry.stage = tabUnusable
		entry.tab = nil
		if tab != nil && refused == nil {
			entry.stage = tabLive
			entry.tab = tab
		}
	}
	return refused
}

func (b *registryBook) closable(id target.ID) bool {
	entry := b.entries[id]
	return entry != nil && entry.stage == tabLive
}
func (b *registryBook) startClosing(id target.ID) *Tab {
	if !b.closable(id) {
		return nil
	}
	entry := b.entries[id]
	entry.stage = tabClosing
	return entry.tab
}
func (b *registryBook) notClosed(id target.ID) {
	if entry := b.entries[id]; entry != nil && entry.stage == tabClosing {
		entry.stage = tabLive
	}
}

// gone retains destroyed IDs so delayed creation events cannot resurrect a page.
func (b *registryBook) gone(id target.ID) (*Tab, bool) {
	if len(b.destroyed) == 1024 {
		b.destroyed = b.destroyed[1:]
	}
	b.destroyed = append(b.destroyed, id)
	entry := b.entries[id]
	delete(b.entries, id)
	if entry == nil {
		return nil, false
	}
	return entry.tab, entry.stage == tabSetup || entry.stage == tabLive || entry.stage == tabClosing
}
func (b *registryBook) counts(id target.ID, entry *registryEntry) bool {
	switch entry.stage {
	case tabLive, tabClosing, tabSetup:
		return true
	case tabPending:
		return b.openers[id] != ""
	case tabUnusable:
		return false
	}
	return false
}
func (b *registryBook) only(id target.ID, holds int) bool {
	if b.creating != 0 || holds != 0 {
		return false
	}
	for other, entry := range b.entries {
		if other != id && b.counts(other, entry) {
			return false
		}
	}
	return true
}
func (b *registryBook) settle(holds int) bool {
	if b.sealed || !b.only("", holds) {
		return false
	}
	b.sealed = true
	return true
}

// listing produces a new immutable registry value in original creation order.
func (b *registryBook) listing() ([]Listed, bool) {
	type orderedTab struct {
		order  int
		listed Listed
	}
	ordered := []orderedTab{}
	registering := b.reconciling
	for id, entry := range b.entries {
		switch entry.stage {
		case tabLive, tabClosing:
			ordered = append(ordered, orderedTab{b.publicIDs[id].order, Listed{Tab: entry.tab, Title: entry.title, URL: entry.url}})
		case tabSetup:
			registering = true
		}
	}
	slices.SortFunc(ordered, func(a, c orderedTab) int { return a.order - c.order })
	listed := make([]Listed, 0, len(ordered))
	for _, item := range ordered {
		listed = append(listed, item.listed)
	}
	return listed, registering
}

func (b *registryBook) name(id target.ID, number uint64) browserop.TabID {
	public := browserop.TabID("t" + strconv.FormatUint(number, 10))
	b.publicIDs[id] = publicTab{id: public, order: len(b.publicIDs)}
	return public
}
func (b *registryBook) opened(opener target.ID) []browserop.TabID {
	ids := []browserop.TabID{}
	for id, parent := range b.openers {
		if parent == opener {
			if public, ok := b.publicIDs[id]; ok {
				ids = append(ids, public.id)
			}
		}
	}
	return ids
}
func (b *registryBook) popups(opener target.ID) ([]browserop.TabID, bool) {
	tabs := []publicTab{}
	for id, entry := range b.entries {
		if b.openers[id] != opener {
			continue
		}
		switch entry.stage {
		case tabPending, tabSetup:
			return nil, false
		case tabLive, tabClosing:
			tabs = append(tabs, b.publicIDs[id])
		}
	}
	slices.SortFunc(tabs, func(a, c publicTab) int { return a.order - c.order })
	ids := make([]browserop.TabID, 0, len(tabs))
	for _, tab := range tabs {
		ids = append(ids, tab.id)
	}
	return ids, true
}
func (b *registryBook) vanished(present map[target.ID]bool) []target.ID {
	var absent []target.ID
	for id := range b.entries {
		if !present[id] {
			absent = append(absent, id)
		}
	}
	return absent
}
