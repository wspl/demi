package plugins

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the API is merged.

import (
	"context"
	"encoding/json"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// User owns one user's plugin instances and enabled set. Share it by pointer;
// methods support concurrent calls. Its shard owns and closes it after stopping
// callers. Command sets keep their instances until the shard closes the user.
type User struct{}

// NewUser creates one instance of every registered plugin for user. The shard
// supplies storage, page marks and product services without exposing its state.
func NewUser(registry *Registry, user webapi.UserID, shard PluginShard) *User {
	panic("not written: b-plugins")
}

// Close ends user streams and calls plugin.Closer on owned instances, joining
// their work before returning. The owner supplies a usable cleanup context.
func (u *User) Close(ctx context.Context) error { panic("not written: b-plugins") }

// Entries returns the registered plugins and their enabled choices for settings.
func (u *User) Entries(ctx context.Context) ([]webapi.PluginEntry, error) {
	panic("not written: b-plugins")
}

// Switch changes the choice, marks the plugin list and state, and ends the
// plugin's streams when disabled. It returns whether the choice changed.
func (u *User) Switch(ctx context.Context, id string, enabled bool) (bool, error) {
	panic("not written: b-plugins")
}

// Toolset contains the commands, profiles and revision a conversation tree takes.
type Toolset struct {
	// Commands includes the product's groups and enabled plugins' commands.
	Commands *host.CommandSet
	// Profiles contains enabled plugins' profiles in registration order.
	Profiles []core.Profile
	// Revision is the enabled command- or profile-contributing ids in registration order.
	Revision string
}

// Toolset composes enabled commands with product groups under demi and profiles.
func (u *User) Toolset(ctx context.Context, product []host.Declared) (Toolset, error) {
	panic("not written: b-plugins")
}

// Revision returns the current toolset revision.
func (u *User) Revision(ctx context.Context) (string, error) { panic("not written: b-plugins") }

// ContextAsk is what a node gives a context source before a provider request.
type ContextAsk struct {
	// Conversation identifies the request's conversation.
	Conversation webapi.ConversationID
	// Node identifies the requesting node.
	Node core.NodeID
	// Cwd is the node's working directory.
	Cwd string
	// Turn is the node's current input turn.
	Turn core.TurnID
	// Seen is the text of the source's own blocks the model receives, oldest first.
	Seen []string
}

// Context asks a source for new context; nil means no new block, including when disabled.
func (u *User) Context(ctx context.Context, id plugin.ID, asked ContextAsk) (*string, error) {
	panic("not written: b-plugins")
}

// Directories pairs a registered plugin with its current Host directory set.
type Directories struct {
	// Plugin identifies the owner of the directories.
	Plugin plugin.ID
	// Directories is empty while the plugin is disabled.
	Directories []plugin.HostDirectory
}

// Directories returns every plugin's Host directories in registration order.
// Disabled plugins have empty sets so host access removes their installations.
func (u *User) Directories(ctx context.Context) ([]Directories, error) {
	panic("not written: b-plugins")
}

// StreamEnd returns a channel closed when the stream's plugin is disabled or
// the user closes. A nil channel means the stream does not exist or is disabled.
func (u *User) StreamEnd(ctx context.Context, name string) (<-chan struct{}, error) {
	panic("not written: b-plugins")
}

// PageStates returns declared user states of enabled plugins keyed by id.
// These values are passed to the product state's generated encoder.
func (u *User) PageStates(ctx context.Context) (map[string]json.RawMessage, error) {
	panic("not written: b-plugins")
}

// PageState returns nil for an absent, disabled or undeclared user state.
func (u *User) PageState(ctx context.Context, id string) (json.RawMessage, error) {
	panic("not written: b-plugins")
}

// ConversationState reads the revision before the state so concurrent changes
// advance beyond the returned revision.
func (u *User) ConversationState(ctx context.Context, id string, conversation webapi.ConversationID) (webapi.PluginStateAnswer, error) {
	panic("not written: b-plugins")
}

// Revisions returns every declared conversation state's revision, including
// disabled plugins, for the conversation summary.
func (u *User) Revisions(conversation webapi.ConversationID) []webapi.PluginRevision {
	panic("not written: b-plugins")
}

// Fire marks each follower's state changed; conversation is nil for user topics.
func (u *User) Fire(topic plugin.Topic, conversation *webapi.ConversationID) {
	panic("not written: b-plugins")
}

// PageCall is a page's call of a plugin method, assembled by the route.
type PageCall struct {
	// Plugin identifies the plugin in the route.
	Plugin string
	// Method identifies the method in the route.
	Method string
	// Params is the JSON body, which the method's schema checks.
	Params json.RawMessage
	// Conversation is the conversation of the route, for a method of that scope.
	Conversation *webapi.ConversationID
}

// PageCall validates parameters against the declared method schema and calls
// the enabled instance. It returns a PageCallError on refusal or plugin failure.
func (u *User) PageCall(ctx context.Context, call PageCall) (json.RawMessage, error) {
	panic("not written: b-plugins")
}
