package plugins

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"sync"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// User owns one user's plugin instances and enabled set. Share it by pointer;
// methods support concurrent calls. Its shard owns and closes it after stopping
// callers. Command sets retain the user host, including while plugins are off.
type User struct {
	notifications      sync.Mutex
	notificationCtx    context.Context
	notificationCancel context.CancelFunc
	notificationCalls  sync.WaitGroup
	registry           *Registry
	id                 webapi.UserID
	shard              Shard
	control            *database.ControlService
	marks              pagesync.UserMarks
	// admission serializes choice reads/commits and instance admission/close across
	// IO. Callers release it before invoking plugins; port operations never take it.
	admission chan struct{}
	enabled   []bool
	instances []*instance
	streams   []chan struct{}
	closed    bool
	// mu protects only conversation revisions; never held during external calls.
	mu        sync.Mutex
	revisions map[webapi.ConversationID][]uint64
}

type instance struct {
	plugin plugin.Plugin
	ctx    context.Context
	cancel context.CancelFunc
	calls  sync.WaitGroup
}

// NewUser creates one instance of every registered plugin for user. The shard
// supplies storage, page marks and product services without exposing its state.
func NewUser(registry *Registry, user webapi.UserID, shard Shard) *User {
	notificationCtx, notificationCancel := context.WithCancel(context.Background())
	u := &User{
		notificationCtx:    notificationCtx,
		notificationCancel: notificationCancel,
		registry:           registry,
		id:                 user,
		shard:              shard,
		control:            shard.Control(),
		marks:              shard.Marks(),
		admission:          make(chan struct{}, 1),
		revisions:          map[webapi.ConversationID][]uint64{},
	}
	for _, registeredPlugin := range registry.plugins {
		u.instances = append(u.instances, newInstance(registeredPlugin.factory))
		u.streams = append(u.streams, make(chan struct{}))
	}
	return u
}

// newInstance gives the user's plugin instance an explicit cancellation owner.
func newInstance(factory plugin.Factory) *instance {
	ctx, cancel := context.WithCancel(context.Background())
	return &instance{plugin: factory.Instance(), ctx: ctx, cancel: cancel}
}

// enter serializes plugin choices and instance ownership without a mutex over IO.
func (u *User) enter(ctx context.Context) error {
	select {
	case u.admission <- struct{}{}:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// leave releases the user's plugin admission permit.
func (u *User) leave() {
	<-u.admission
}

// close ends admitted calls before asking a plugin to join its retained work.
func (i *instance) close() {
	if i == nil {
		return
	}
	i.cancel()
	i.calls.Wait()
	if closer, ok := i.plugin.(plugin.Closer); ok {
		closer.Close()
	}
}

// Close ends user streams and calls plugin.Closer on owned instances, joining
// their work before returning. The owner supplies a usable cleanup context.
func (u *User) Close(ctx context.Context) error {
	u.notifications.Lock()
	u.notificationCancel()
	u.notifications.Unlock()
	u.notificationCalls.Wait()
	if err := u.enter(ctx); err != nil {
		return err
	}
	defer u.leave()
	if u.closed {
		return nil
	}
	u.closed = true
	for i, instance := range u.instances {
		close(u.streams[i])
		if instance != nil {
			instance.cancel()
		}
	}
	for i, instance := range u.instances {
		instance.close()
		u.instances[i] = nil
	}
	return nil
}

// choices reads the durable choices once while the admission permit is held.
func (u *User) choices(ctx context.Context) error {
	if u.closed {
		return context.Canceled
	}
	if u.enabled != nil {
		return nil
	}
	choices, err := u.control.UserPlugins(ctx, u.id)
	if err != nil {
		return err
	}
	u.enabled = make([]bool, len(u.registry.plugins))
	for i, registeredPlugin := range u.registry.plugins {
		enabled, found := choices[string(registeredPlugin.manifest.ID)]
		u.enabled[i] = !found || enabled
	}
	return nil
}

// enabledSet takes a consistent snapshot of the user's plugin choices.
func (u *User) enabledSet(ctx context.Context) ([]bool, error) {
	if err := u.enter(ctx); err != nil {
		return nil, err
	}
	defer u.leave()
	if err := u.choices(ctx); err != nil {
		return nil, err
	}
	return slices.Clone(u.enabled), nil
}

// Entries returns the registered plugins and their enabled choices for settings.
func (u *User) Entries(ctx context.Context) ([]webapi.PluginEntry, error) {
	enabled, err := u.enabledSet(ctx)
	if err != nil {
		return nil, err
	}
	entries := make([]webapi.PluginEntry, 0, len(enabled))
	for i, registeredPlugin := range u.registry.plugins {
		entries = append(
			entries,
			webapi.PluginEntry{
				ID:          string(registeredPlugin.manifest.ID),
				Name:        registeredPlugin.manifest.Name,
				Description: registeredPlugin.manifest.Description,
				Enabled:     enabled[i],
				Packages:    slices.Clone(registeredPlugin.packages),
			},
		)
	}
	return entries, nil
}

// Switch changes the choice, marks the plugin list and state, and ends the
// plugin's streams when disabled. It returns whether the choice changed.
func (u *User) Switch(ctx context.Context, id string, enabled bool) (bool, error) {
	index, registeredPlugin := u.registry.lookup(id)
	if registeredPlugin == nil {
		return false, unknownPlugin(id)
	}
	if err := u.enter(ctx); err != nil {
		return false, err
	}
	defer u.leave()
	if err := u.choices(ctx); err != nil {
		return false, err
	}
	if u.enabled[index] == enabled {
		return false, nil
	}
	// Once the durable choice commits, publish and clean up even if its caller leaves.
	if err := u.control.SetUserPlugin(context.WithoutCancel(ctx), u.id, id, enabled); err != nil {
		return false, err
	}
	u.enabled[index] = enabled
	if !enabled {
		close(u.streams[index])
		u.streams[index] = make(chan struct{})
		u.instances[index].close()
		u.instances[index] = nil
	}
	u.marks.Mark(pagesync.Part{Kind: pagesync.Plugins})
	u.marks.Mark(pagesync.Part{Kind: pagesync.Plugin, PluginID: id})
	return true, nil
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
	enabled, err := u.enabledSet(ctx)
	if err != nil {
		return Toolset{}, err
	}
	commands, err := u.registry.compose(u, product, enabled)
	if err != nil {
		return Toolset{}, err
	}
	profiles := []core.Profile{}
	for i, registeredPlugin := range u.registry.plugins {
		if enabled[i] {
			profiles = append(profiles, copyProfiles(registeredPlugin.manifest.Profiles)...)
		}
	}
	return Toolset{
		Commands: commands,
		Profiles: profiles,
		Revision: u.revisionOf(enabled),
	}, nil
}

// revisionOf identifies the enabled command/profile contributions in order.
func (u *User) revisionOf(enabled []bool) string {
	ids := []string{}
	for i, registeredPlugin := range u.registry.plugins {
		contributesTools := len(registeredPlugin.manifest.Commands) != 0 || len(registeredPlugin.manifest.Profiles) != 0
		if enabled[i] && contributesTools {
			ids = append(ids, string(registeredPlugin.manifest.ID))
		}
	}
	return strings.Join(ids, ",")
}

// Revision returns the current toolset revision.
func (u *User) Revision(ctx context.Context) (string, error) {
	enabled, err := u.enabledSet(ctx)
	if err != nil {
		return "", err
	}
	return u.revisionOf(enabled), nil
}

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
	index, registeredPlugin := u.registry.lookup(string(id))
	if registeredPlugin == nil {
		return nil, nil
	}
	reply, err := u.request(
		ctx,
		index,
		&plugin.RequestContext{
			User:         u.id,
			Conversation: asked.Conversation,
			Node:         asked.Node,
			CWD:          asked.Cwd,
			Turn:         asked.Turn,
			Seen:         slices.Clone(asked.Seen),
		},
		&asked.Conversation,
		nil,
	)
	if err != nil {
		return nil, err
	}
	if reply == nil {
		return nil, nil
	}
	if contextReply, ok := reply.(*plugin.ReplyContext); ok {
		return contextReply.Text, nil
	}
	return nil, wrongReply("a context request", reply)
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
	enabled, err := u.enabledSet(ctx)
	if err != nil {
		return nil, err
	}
	stored, err := u.control.PluginDirectories(ctx, u.id)
	if err != nil {
		return nil, err
	}
	directories := make([]Directories, 0, len(enabled))
	for i, registeredPlugin := range u.registry.plugins {
		set := []plugin.HostDirectory{}
		if enabled[i] {
			if value := stored[string(registeredPlugin.manifest.ID)]; value != nil {
				set = value
			}
		}
		directories = append(directories, Directories{Plugin: registeredPlugin.manifest.ID, Directories: set})
	}
	return directories, nil
}

// StreamEnd returns a channel closed when the stream's plugin is disabled or
// the user closes. A nil channel means the stream does not exist or is disabled.
func (u *User) StreamEnd(ctx context.Context, name string) (<-chan struct{}, error) {
	if err := u.enter(ctx); err != nil {
		return nil, err
	}
	defer u.leave()
	if err := u.choices(ctx); err != nil {
		return nil, err
	}
	for i, registeredPlugin := range u.registry.plugins {
		if u.enabled[i] {
			for _, stream := range registeredPlugin.manifest.Streams {
				if stream.Name == name {
					return u.streams[i], nil
				}
			}
		}
	}
	return nil, nil
}

// PageStates returns declared user states of enabled plugins keyed by id.
// These values are passed to the product state's generated encoder.
func (u *User) PageStates(ctx context.Context) (map[string]json.RawMessage, error) {
	states := map[string]json.RawMessage{}
	for _, registeredPlugin := range u.registry.plugins {
		state, err := u.PageState(ctx, string(registeredPlugin.manifest.ID))
		if err != nil {
			return nil, err
		}
		if state != nil {
			states[string(registeredPlugin.manifest.ID)] = state
		}
	}
	return states, nil
}

// PageState returns nil for an absent, disabled or undeclared user state.
func (u *User) PageState(ctx context.Context, id string) (json.RawMessage, error) {
	index, registeredPlugin := u.registry.lookup(id)
	if registeredPlugin == nil || registeredPlugin.manifest.Page == nil || registeredPlugin.manifest.Page.User == nil {
		return nil, nil
	}
	return u.stateOf(ctx, index, nil)
}

// ConversationState reads the revision before the state so concurrent changes
// advance beyond the returned revision.
func (u *User) ConversationState(
	ctx context.Context,
	id string,
	conversation webapi.ConversationID,
) (webapi.PluginStateAnswer, error) {
	index, registeredPlugin := u.registry.lookup(id)
	if registeredPlugin == nil || registeredPlugin.manifest.Page == nil ||
		registeredPlugin.manifest.Page.Conversation == nil {
		return webapi.PluginStateAnswer{}, unknownPlugin(id)
	}
	revision := u.conversationRevision(index, conversation)
	state, err := u.stateOf(ctx, index, &conversation)
	if err != nil {
		return webapi.PluginStateAnswer{}, &PageCallError{Kind: PluginFailed, Err: err}
	}
	if state == nil {
		return webapi.PluginStateAnswer{}, &PageCallError{Kind: Disabled, Plugin: id}
	}
	return webapi.PluginStateAnswer{Revision: revision, State: state}, nil
}

// stateOf dispatches the page's state request in its declared scope.
func (u *User) stateOf(
	ctx context.Context,
	index int,
	conversation *webapi.ConversationID,
) (json.RawMessage, error) {
	reply, err := u.request(
		ctx,
		index,
		&plugin.RequestPageState{User: u.id, Conversation: conversation},
		conversation,
		nil,
	)
	if err != nil || reply == nil {
		return nil, err
	}
	if state, ok := reply.(*plugin.ReplyState); ok {
		return state.State, nil
	}
	return nil, wrongReply("its page state", reply)
}

// conversationRevision reads one plugin's conversation revision under the revision mutex.
func (u *User) conversationRevision(index int, conversation webapi.ConversationID) uint64 {
	u.mu.Lock()
	defer u.mu.Unlock()
	if revisions := u.revisions[conversation]; revisions != nil {
		return revisions[index]
	}
	return 0
}

// Revisions returns every declared conversation state's revision, including
// disabled plugins, for the conversation summary.
func (u *User) Revisions(conversation webapi.ConversationID) []webapi.PluginRevision {
	revisions := []webapi.PluginRevision{}
	for i, registeredPlugin := range u.registry.plugins {
		if registeredPlugin.manifest.Page != nil && registeredPlugin.manifest.Page.Conversation != nil {
			revisions = append(
				revisions,
				webapi.PluginRevision{
					Plugin:   string(registeredPlugin.manifest.ID),
					Revision: u.conversationRevision(i, conversation),
				},
			)
		}
	}
	return revisions
}

// Fire marks each follower's state changed; conversation is nil for user topics.
func (u *User) Fire(topic plugin.Topic, conversation *webapi.ConversationID) {
	if conversation != nil {
		id := *conversation
		conversation = &id
	}
	for _, id := range u.registry.Followers(topic) {
		index, _ := u.registry.lookup(string(id))
		u.changed(index, conversation)
	}
	for i, p := range u.registry.plugins {
		if p.manifest.Page != nil && slices.Contains(p.manifest.Page.Told, topic) {
			u.notify(i, &plugin.RequestTopic{User: u.id, Topic: topic, Conversation: conversation}, conversation)
		}
	}
}

// changed publishes one plugin's state invalidation after releasing the mutex.
func (u *User) changed(index int, conversation *webapi.ConversationID) {
	part := pagesync.Part{
		Kind:     pagesync.Plugin,
		PluginID: string(u.registry.plugins[index].manifest.ID),
	}
	if conversation != nil {
		u.mu.Lock()
		if u.revisions[*conversation] == nil {
			u.revisions[*conversation] = make([]uint64, len(u.registry.plugins))
		}
		u.revisions[*conversation][index]++
		u.mu.Unlock()
		part = pagesync.Part{Kind: pagesync.Conversation, ConversationID: *conversation}
	}
	u.marks.Mark(part)
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
// the enabled instance. It returns ErrUnknownPlugin or a PageCallError on refusal or plugin failure.
func (u *User) PageCall(ctx context.Context, call PageCall) (json.RawMessage, error) {
	index, registeredPlugin := u.registry.lookup(call.Plugin)
	if registeredPlugin == nil {
		return nil, unknownPlugin(call.Plugin)
	}
	enabled, err := u.enabledSet(ctx)
	if err != nil {
		return nil, &PageCallError{Kind: PluginFailed, Err: err}
	}
	if !enabled[index] {
		return nil, &PageCallError{Kind: Disabled, Plugin: call.Plugin}
	}
	scope := plugin.ScopeUser
	if call.Conversation != nil {
		scope = plugin.ScopeConversation
	}
	method := pageMethod(registeredPlugin.manifest.Page, call.Method, scope)
	if method == nil {
		return nil, &PageCallError{
			Kind:   UnknownMethod,
			Plugin: call.Plugin,
			Method: call.Method,
		}
	}
	if err := method.Params.Check(call.Params); err != nil {
		return nil, &PageCallError{Kind: InvalidParams, Message: err.Error(), Err: err}
	}
	if trimmed := bytes.TrimSpace(call.Params); len(trimmed) == 0 || trimmed[0] != '{' {
		return nil, &PageCallError{
			Kind:    InvalidParams,
			Message: "the parameters are not an object",
		}
	}
	reply, err := u.request(
		ctx,
		index,
		&plugin.RequestPageCall{
			User:         u.id,
			Method:       call.Method,
			Params:       slices.Clone(call.Params),
			Conversation: call.Conversation,
		},
		call.Conversation,
		nil,
	)
	if err != nil {
		return nil, &PageCallError{Kind: PluginFailed, Err: err}
	}
	if reply == nil {
		return nil, &PageCallError{Kind: Disabled, Plugin: call.Plugin}
	}
	if result, ok := reply.(*plugin.ReplyResult); ok {
		return result.Result, nil
	}
	return nil, &PageCallError{Kind: PluginFailed, Err: wrongReply("a page call", reply)}
}

// request admits calls atomically with a switch, then releases admission before
// plugin code runs. RPC commands of old trees remain callable while disabled.
func (u *User) request(
	ctx context.Context,
	index int,
	request plugin.Request,
	conversation *webapi.ConversationID,
	rpc *host.RPCPort,
) (plugin.Reply, error) {
	if err := u.enter(ctx); err != nil {
		return nil, err
	}
	if err := u.choices(ctx); err != nil {
		u.leave()
		return nil, err
	}
	if rpc == nil && !u.enabled[index] {
		u.leave()
		return nil, nil
	}
	if u.instances[index] == nil {
		u.instances[index] = newInstance(u.registry.plugins[index].factory)
	}
	instance := u.instances[index]
	instance.calls.Add(1)
	u.leave()
	defer instance.calls.Done()
	callCtx, cancel := context.WithCancel(ctx)
	stop := context.AfterFunc(instance.ctx, cancel)
	defer stop()
	defer cancel()
	return instance.plugin.Call(
		callCtx,
		request,
		plugin.NewPort(
			requestPort{
				user:         u,
				index:        index,
				conversation: conversation,
				rpc:          rpc,
				lifetime:     instance.ctx,
			},
		),
	)
}

// wrongReply reports a plugin reply that does not match its request.
func wrongReply(request string, reply plugin.Reply) error {
	return &plugin.ErrorFailed{
		Message: fmt.Sprintf("the plugin answered %s with %v", request, reply),
	}
}

// pageMethod finds the page method declared for the requested name and scope.
func pageMethod(page *plugin.Page, name string, scope plugin.Scope) *plugin.Method {
	var method *plugin.Method
	if page != nil {
		for i := range page.Methods {
			declaredMethod := &page.Methods[i]
			if declaredMethod.Name == name && declaredMethod.Scope == scope {
				method = declaredMethod
				break
			}
		}
	}
	return method
}
