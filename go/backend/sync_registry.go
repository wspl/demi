package backend

import (
	"cmp"
	"context"
	"maps"
	"slices"
	"sync"

	"github.com/wspl/demi/go/backend/storage"
	"github.com/wspl/demi/go/webapi"
)

// SyncPart is a part of the product state that a page shows (backend.md §
// Browser synchronization). A channel sends the parts that changed in their
// order, so a new conversation's summary goes before the order that places
// it.
type SyncPart struct {
	kind syncPartKind
	// conversation is the conversation of a summary part.
	conversation webapi.ConversationID
}

// syncPartKind is a part's kind, in the order parts are sent.
type syncPartKind uint8

const (
	syncConversation syncPartKind = iota
	syncConversationOrder
	syncPreferences
	syncUser
	syncWorkspaces
	syncDevices
	syncExposes
	syncProviders
	syncCloud
)

// The parts that are one per user.
var (
	// SyncConversationOrder is the order of every conversation.
	SyncConversationOrder = SyncPart{kind: syncConversationOrder}
	SyncPreferences       = SyncPart{kind: syncPreferences}
	SyncUser              = SyncPart{kind: syncUser}
	SyncWorkspaces        = SyncPart{kind: syncWorkspaces}
	SyncDevices           = SyncPart{kind: syncDevices}
	SyncExposes           = SyncPart{kind: syncExposes}
	SyncProviders         = SyncPart{kind: syncProviders}
	SyncCloud             = SyncPart{kind: syncCloud}
)

// SyncConversation is the summary of conversation id.
func SyncConversation(id webapi.ConversationID) SyncPart {
	return SyncPart{kind: syncConversation, conversation: id}
}

// Conversation is the conversation of a summary part, and false for any
// other part.
func (p SyncPart) Conversation() (webapi.ConversationID, bool) {
	return p.conversation, p.kind == syncConversation
}

// compareSyncParts orders parts as they are sent.
func compareSyncParts(a, b SyncPart) int {
	return cmp.Or(cmp.Compare(a.kind, b.kind), cmp.Compare(a.conversation.String(), b.conversation.String()))
}

// SyncMarked is what was marked on a channel since its task last took it.
type SyncMarked struct {
	// Parts are the changed parts, each once, in the order they are sent.
	Parts []SyncPart
	// SessionEnded says the session the channel opened with was signed out.
	SessionEnded bool
}

// SyncRegistry is the registry of each user's open synchronization channels,
// a shared service (backend.md § Browser synchronization). Whoever commits a
// change a page shows marks the changed part in it, from any goroutine: the
// mark adds the part to the set of each of the user's channels and wakes the
// channel's task, which reads the part in the shard when it can send it. A
// channel holds at most one mark per part however far its page falls behind,
// so it never queues. Its zero value has no channel open.
type SyncRegistry struct {
	mu       sync.Mutex
	channels map[webapi.UserID][]*syncChannelMarks
}

// syncChannelMarks is one open channel as the registry marks it.
type syncChannelMarks struct {
	// session is the session the channel opened with.
	session storage.TokenHash
	mu      sync.Mutex
	parts   map[SyncPart]struct{}
	ended   bool
	// wake holds one signal after a mark, so a mark made while the task is
	// busy is not missed.
	wake chan struct{}
}

// mark adds part to the channel's set and wakes its task.
func (c *syncChannelMarks) mark(part SyncPart) {
	c.mu.Lock()
	c.parts[part] = struct{}{}
	c.mu.Unlock()
	c.signal()
}

// endSession records that the channel's session was signed out and wakes
// its task.
func (c *syncChannelMarks) endSession() {
	c.mu.Lock()
	c.ended = true
	c.mu.Unlock()
	c.signal()
}

// signal wakes the channel's task, or leaves the signal for its next wait.
func (c *syncChannelMarks) signal() {
	select {
	case c.wake <- struct{}{}:
	default:
	}
}

// pending says whether anything was marked since the last take.
func (c *syncChannelMarks) pending() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	return len(c.parts) > 0 || c.ended
}

// Register registers a channel of user's page, opened with the session
// session, for every change marked from now on, until the registration is
// closed.
func (r *SyncRegistry) Register(user webapi.UserID, session storage.TokenHash) *SyncRegistration {
	channel := &syncChannelMarks{session: session, parts: map[SyncPart]struct{}{}, wake: make(chan struct{}, 1)}
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.channels == nil {
		r.channels = map[webapi.UserID][]*syncChannelMarks{}
	}
	r.channels[user] = append(r.channels[user], channel)
	return &SyncRegistration{registry: r, user: user, channel: channel}
}

// Mark marks part changed on each open channel of user.
func (r *SyncRegistry) Mark(user webapi.UserID, part SyncPart) {
	r.mu.Lock()
	defer r.mu.Unlock()
	for _, channel := range r.channels[user] {
		channel.mark(part)
	}
}

// MarkEveryone marks part changed on every user's open channels, for a
// change every user sees, such as a shared instance's provider entry.
func (r *SyncRegistry) MarkEveryone(part SyncPart) {
	r.mu.Lock()
	defer r.mu.Unlock()
	for _, channels := range r.channels {
		for _, channel := range channels {
			channel.mark(part)
		}
	}
}

// EndSession ends the open channels of user that opened with the session
// session, which was signed out.
func (r *SyncRegistry) EndSession(user webapi.UserID, session storage.TokenHash) {
	r.mu.Lock()
	defer r.mu.Unlock()
	for _, channel := range r.channels[user] {
		if channel.session == session {
			channel.endSession()
		}
	}
}

// SyncRegistration is a channel's place in the registry, which Close gives
// up.
type SyncRegistration struct {
	registry *SyncRegistry
	user     webapi.UserID
	channel  *syncChannelMarks
}

// Marked waits until something is marked on the channel, and takes nothing.
// It returns ctx's error if ctx ends first.
func (g *SyncRegistration) Marked(ctx context.Context) error {
	for !g.channel.pending() {
		select {
		case <-g.channel.wake:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	return nil
}

// Take takes what was marked since the last take.
func (g *SyncRegistration) Take() SyncMarked {
	g.channel.mu.Lock()
	defer g.channel.mu.Unlock()
	marked := SyncMarked{Parts: slices.SortedFunc(maps.Keys(g.channel.parts), compareSyncParts), SessionEnded: g.channel.ended}
	clear(g.channel.parts)
	g.channel.ended = false
	return marked
}

// Close removes the channel from the registry: nothing is marked on it
// afterwards.
func (g *SyncRegistration) Close() {
	r := g.registry
	r.mu.Lock()
	defer r.mu.Unlock()
	open := slices.DeleteFunc(r.channels[g.user], func(channel *syncChannelMarks) bool { return channel == g.channel })
	if len(open) == 0 {
		delete(r.channels, g.user)
		return
	}
	r.channels[g.user] = open
}
