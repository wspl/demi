package pagesync

import (
	"cmp"
	"context"
	"slices"
	"sync"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/webapi"
)

// SyncRegistry tracks every user's open channels. Its zero value is ready for
// use. Share it by pointer; it must not be copied after first use.
type SyncRegistry struct {
	// mu protects membership and every registration's pending marks and wake.
	mu       sync.Mutex
	channels map[webapi.UserID]map[*Registration]struct{}
}

// Marked contains the changes since the last Take, in delivery order.
type Marked struct {
	Parts []Part
	// SessionEnded means the session the channel opened with was signed out.
	SessionEnded bool
}

// Registration is a channel's place in the registry. Its owner defers Release
// and cancels and joins any Marked call before releasing it.
type Registration struct {
	registry     *SyncRegistry
	user         webapi.UserID
	session      database.TokenHash
	parts        map[Part]struct{}
	sessionEnded bool
	wake         chan struct{}
}

// Register registers a user's channel for changes marked until Release.
func (r *SyncRegistry) Register(user webapi.UserID, session database.TokenHash) *Registration {
	c := &Registration{registry: r, user: user, session: session, wake: make(chan struct{})}
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.channels == nil {
		r.channels = make(map[webapi.UserID]map[*Registration]struct{})
	}
	if r.channels[user] == nil {
		r.channels[user] = make(map[*Registration]struct{})
	}
	r.channels[user][c] = struct{}{}
	return c
}

// UserMarks marks one user's changes without holding a shard reference.
// Copies share the same registry.
type UserMarks struct {
	registry *SyncRegistry
	user     webapi.UserID
}

// Of returns the marks of the user's changes.
func (r *SyncRegistry) Of(user webapi.UserID) UserMarks {
	return UserMarks{registry: r, user: user}
}

// Mark marks the part changed on the user's open channels.
func (m UserMarks) Mark(part Part) { m.registry.Mark(m.user, part) }

// Mark marks the part changed on each open channel of the user.
func (r *SyncRegistry) Mark(user webapi.UserID, part Part) {
	r.change(&user, nil, &part)
}

// MarkEveryone marks a change that every user sees.
func (r *SyncRegistry) MarkEveryone(part Part) { r.change(nil, nil, &part) }

// EndSession marks the user's channels opened with the signed-out session.
func (r *SyncRegistry) EndSession(user webapi.UserID, session database.TokenHash) {
	r.change(&user, &session, nil)
}

// change records a page-visible change and collects notifications under the
// registry lock, then delivers them outside it. Each replaced wake has one owner.
func (r *SyncRegistry) change(user *webapi.UserID, session *database.TokenHash, part *Part) {
	var wakes []chan struct{}
	r.mu.Lock()
	for id, channels := range r.channels {
		if user != nil && id != *user {
			continue
		}
		for c := range channels {
			if session != nil && c.session != *session {
				continue
			}
			if part != nil {
				if c.parts == nil {
					c.parts = make(map[Part]struct{})
				}
				c.parts[*part] = struct{}{}
			} else {
				c.sessionEnded = true
			}
			wakes = append(wakes, c.wake)
			c.wake = make(chan struct{})
		}
	}
	r.mu.Unlock()
	for _, wake := range wakes {
		close(wake)
	}
}

// Marked waits until something is marked and takes nothing. Cancellation leaves
// pending marks intact. It creates no goroutines.
func (c *Registration) Marked(ctx context.Context) error {
	for {
		c.registry.mu.Lock()
		ready := len(c.parts) != 0 || c.sessionEnded
		wake := c.wake
		c.registry.mu.Unlock()
		if ready {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-wake:
		}
	}
}

// Take takes what was marked since the last take, with summaries before order.
func (c *Registration) Take() Marked {
	c.registry.mu.Lock()
	parts := c.parts
	marked := Marked{SessionEnded: c.sessionEnded}
	c.parts = nil
	c.sessionEnded = false
	c.registry.mu.Unlock()
	for part := range parts {
		marked.Parts = append(marked.Parts, part)
	}
	slices.SortFunc(marked.Parts, func(a, b Part) int {
		if order := cmp.Compare(a.Kind, b.Kind); order != 0 {
			return order
		}
		if order := cmp.Compare(a.ConversationID, b.ConversationID); order != 0 {
			return order
		}
		return cmp.Compare(a.PluginID, b.PluginID)
	})
	return marked
}

// Release unregisters the channel. Repeated calls are harmless.
func (c *Registration) Release() {
	r := c.registry
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.channels[c.user], c)
	if len(r.channels[c.user]) == 0 {
		delete(r.channels, c.user)
	}
}
