package pluginhost

import (
	"fmt"
	"log/slog"
	"slices"

	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/types"
)

// Registry holds the backend's plugins in registration order, shared by users.
// Construct it with NewRegistry; its published declarations are immutable.
type Registry struct{ plugins []registered }

type registered struct {
	factory  plugin.Factory
	manifest plugin.Manifest
	packages []string
}

// NewRegistry checks factories in order. Contributions bound to operations that
// serves refuses are left out and logged; an invalid manifest refuses startup.
func NewRegistry(factories []plugin.Factory, serves func(commanddecl.NativeOperation) bool) (*Registry, error) {
	r := &Registry{}
	ids := map[string]bool{}
	profiles := map[string]bool{}
	streams := map[string]bool{}
	pages := map[string]bool{}
	for _, factory := range factories {
		manifest := factory.Manifest()
		id := string(manifest.ID)
		if ids[id] {
			return nil, fmt.Errorf("two plugins have the id \"%s\"", manifest.ID)
		}
		ids[id] = true
		if err := r.checkPanelKinds(manifest); err != nil {
			return nil, err
		}
		if err := checkManifestContributions(manifest, profiles, streams, pages); err != nil {
			return nil, err
		}
		// The generated codec detaches the manifest and refuses values a manifest
		// must not hold (notably the reserved execution id).
		data, err := manifest.MarshalJSON()
		if err != nil {
			return nil, fmt.Errorf("plugin \"%s\"'s commands are refused: %w", manifest.ID, err)
		}
		detached, err := plugin.DecodeManifest(data)
		if err != nil {
			return nil, fmt.Errorf("plugin \"%s\"'s commands are refused: %w", manifest.ID, err)
		}
		r.plugins = append(r.plugins, registered{factory: factory, manifest: detached})
	}
	// Check unfiltered declarations too: a deployment's catalog cannot hide a
	// conflict that a different deployment would expose.
	if _, err := r.compose(nil, nil, nil); err != nil {
		return nil, err
	}
	for i, p := range r.plugins {
		r.plugins[i] = served(p.factory, p.manifest, serves)
	}
	return r, nil
}

// ContextSources returns the context sources in registration order.
func (r *Registry) ContextSources() []plugin.ID {
	sources := []plugin.ID{}
	for _, p := range r.plugins {
		if p.manifest.Context {
			sources = append(sources, p.manifest.ID)
		}
	}
	return sources
}

// Profiles returns the plugins' profiles in registration order.
func (r *Registry) Profiles() []types.Profile {
	profiles := []types.Profile{}
	for _, p := range r.plugins {
		profiles = append(profiles, copyProfiles(p.manifest.Profiles)...)
	}
	return profiles
}

// Streams returns every user stream the startup catalog serves, in registration order.
func (r *Registry) Streams() []plugin.Stream {
	streams := []plugin.Stream{}
	for _, p := range r.plugins {
		for _, stream := range p.manifest.Streams {
			stream.Constants = slices.Clone(stream.Constants)
			for i := range stream.Constants {
				stream.Constants[i].Value = slices.Clone(stream.Constants[i].Value)
			}
			streams = append(streams, stream)
		}
	}
	return streams
}

// Followers returns the plugins whose page state follows topic, in registration order.
func (r *Registry) Followers(topic plugin.Topic) []plugin.ID {
	followers := []plugin.ID{}
	for _, p := range r.plugins {
		if p.manifest.Page != nil {
			if state := p.manifest.Page.State(topic.Scope()); state != nil && slices.Contains(state.Topics, topic) {
				followers = append(followers, p.manifest.ID)
			}
		}
	}
	return followers
}

// lookup finds the registered plugin by its route or source id.
func (r *Registry) lookup(id string) (int, *registered) {
	for i := range r.plugins {
		if string(r.plugins[i].manifest.ID) == id {
			return i, &r.plugins[i]
		}
	}
	return -1, nil
}

// served keeps whole contributions only when the startup catalog serves them.
func served(
	factory plugin.Factory,
	manifest plugin.Manifest,
	serves func(commanddecl.NativeOperation) bool,
) registered {
	packages := map[string]bool{}
	keep := func(what, name string, operations []commanddecl.NativeOperation) bool {
		for _, operation := range operations {
			if !serves(operation) {
				slog.Info(
					"a part whose package the catalog does not serve is left out",
					"plugin",
					manifest.ID,
					"what",
					what,
					"name",
					name,
				)
				return false
			}
		}
		for _, operation := range operations {
			packages[operation.Package] = true
		}
		return true
	}
	manifest.Commands = slices.DeleteFunc(manifest.Commands, func(c plugin.Commands) bool {
		return !keep("command", commanddecl.Name(c.Tree.Node), nativeOperations(c.Tree.Node))
	})
	manifest.Streams = slices.DeleteFunc(manifest.Streams, func(s plugin.Stream) bool {
		return !keep("user stream", s.Name, []commanddecl.NativeOperation{s.Operation})
	})
	if page := manifest.Page; page != nil {
		if page.User != nil && !keep("page state", "user", page.User.Operations) {
			page.User = nil
		}
		if page.Conversation != nil && !keep("page state", "conversation", page.Conversation.Operations) {
			page.Conversation = nil
		}
		page.Methods = slices.DeleteFunc(
			page.Methods,
			func(m plugin.Method) bool {
				return !keep("page method", m.Name, m.Operations)
			},
		)
	}
	names := make([]string, 0, len(packages))
	for name := range packages {
		names = append(names, name)
	}
	slices.Sort(names)
	return registered{factory: factory, manifest: manifest, packages: names}
}

// nativeOperations collects the package operations of one plugin command tree.
func nativeOperations(tree commanddecl.Node[commanddecl.NativeOperation]) []commanddecl.NativeOperation {
	var operations []commanddecl.NativeOperation
	switch n := tree.(type) {
	case *commanddecl.Leaf[commanddecl.NativeOperation]:
		switch kind := n.Kind.(type) {
		case *commanddecl.Native[commanddecl.NativeOperation]:
			operations = append(operations, kind.Binding)
		case *commanddecl.RPC[commanddecl.NativeOperation]:
		}
	case *commanddecl.Group[commanddecl.NativeOperation]:
		for _, child := range n.Subcommands {
			operations = append(operations, nativeOperations(child)...)
		}
	}
	return operations
}

// compose binds plugin trees to a user's instances and checks platform ownership.
func (r *Registry) compose(user *User, product []host.Declared, enabled []bool) (*host.CommandSet, error) {
	set := &host.CommandSet{}
	demi := len(product) > 0
	if demi {
		if err := set.Register(host.Group(plugin.DemiRoot, plugin.DemiSummary, product...)); err != nil {
			return nil, fmt.Errorf("the product's groups are refused: %w", err)
		}
	}
	names := map[string]bool{"agent": true, "shell": true, "host": true}
	for i, p := range r.plugins {
		if enabled != nil && !enabled[i] {
			continue
		}
		for _, c := range p.manifest.Commands {
			name := commanddecl.Name(c.Tree.Node)
			strip := 0
			if c.Placement == plugin.PlacementDemi {
				if names[name] {
					return nil, fmt.Errorf("plugin \"%s\" declares \"%s\", which is taken", p.manifest.ID, "demi "+name)
				}
				names[name] = true
				strip = 1
			} else if name == plugin.DemiRoot {
				return nil, fmt.Errorf("plugin \"%s\" declares \"%s\", which is taken", p.manifest.ID, name)
			}
			tree := host.Served(c.Tree.Node, forward{user: user, index: i, strip: strip})
			var err error
			if c.Placement == plugin.PlacementRoot {
				err = set.Register(tree)
			} else if demi {
				err = set.Graft([]string{plugin.DemiRoot}, tree)
			} else {
				err = set.Register(host.Group(plugin.DemiRoot, plugin.DemiSummary, tree))
				demi = true
			}
			if err != nil {
				return nil, fmt.Errorf("plugin \"%s\"'s commands are refused: %w", p.manifest.ID, err)
			}
		}
	}
	return set, nil
}

// copyProfiles detaches mutable profile fields returned to a conversation tree.
func copyProfiles(profiles []types.Profile) []types.Profile {
	copied := slices.Clone(profiles)
	for i := range copied {
		p := &copied[i]
		if p.Instructions != nil {
			p.Instructions = new(*p.Instructions)
		}
		if p.Model != nil {
			p.Model = copyProfileModel(*p.Model)
		}
		if p.Commands != nil {
			paths := slices.Clone(*p.Commands)
			for j := range paths {
				paths[j] = slices.Clone(paths[j])
			}
			p.Commands = &paths
		}
	}
	return copied
}

// copyProfileModel detaches a profile's selected model and thinking settings.
func copyProfileModel(selection types.ModelSelection) *types.ModelSelection {
	if selection.ServiceTierID != nil {
		selection.ServiceTierID = new(*selection.ServiceTierID)
	}
	model := &selection.Model
	if model.InputLimit != nil {
		model.InputLimit = new(*model.InputLimit)
	}
	if model.OutputLimit != nil {
		model.OutputLimit = new(*model.OutputLimit)
	}
	if model.AcceptedExtensions != nil {
		model.AcceptedExtensions = new(slices.Clone(*model.AcceptedExtensions))
	}
	copyModelCapabilities(model)
	switch config := selection.Thinking.(type) {
	case *types.AdaptiveConfig:
		selection.Thinking = new(*config)
	case *types.BudgetConfig:
		selection.Thinking = new(*config)
	case *types.EffortConfig:
		value := *config
		if value.Summary != nil {
			value.Summary = new(*value.Summary)
		}
		selection.Thinking = &value
	case *types.DisabledConfig:
		selection.Thinking = &types.DisabledConfig{}
	}
	return &selection
}

// checkManifestContributions checks ownership of profiles, streams and page topics.
func checkManifestContributions(manifest plugin.Manifest, profiles, streams, pages map[string]bool) error {
	for _, profile := range manifest.Profiles {
		reason := ""
		if profile.Name == types.ProfileInherit {
			reason = "is reserved for inheriting the parent"
		} else if profiles[profile.Name] {
			reason = "another plugin declares"
		}
		if reason != "" {
			return fmt.Errorf("plugin \"%s\" declares the profile \"%s\", which %s", manifest.ID, profile.Name, reason)
		}
		profiles[profile.Name] = true
	}
	for _, stream := range manifest.Streams {
		if streams[stream.Name] {
			return fmt.Errorf("plugin \"%s\" declares the user stream \"%s\", which is taken", manifest.ID, stream.Name)
		}
		streams[stream.Name] = true
	}
	if page := manifest.Page; page != nil {
		if pages[page.Package] {
			return fmt.Errorf("plugin \"%s\"'s page package \"%s\" is another plugin's", manifest.ID, page.Package)
		}
		pages[page.Package] = true
		return checkPageTopics(manifest.ID, page)
	}
	return nil
}

// copyModelCapabilities detaches the model’s mutable thinking capabilities.
func copyModelCapabilities(model *types.Model) {
	model.Thinking = slices.Clone(model.Thinking)
	for i, capability := range model.Thinking {
		switch c := capability.(type) {
		case *types.AdaptiveCapability:
			value := *c
			value.Efforts = slices.Clone(c.Efforts)
			if c.DefaultEffort != nil {
				value.DefaultEffort = new(*c.DefaultEffort)
			}
			model.Thinking[i] = &value
		case *types.BudgetCapability:
			value := *c
			if c.MinBudgetTokens != nil {
				value.MinBudgetTokens = new(*c.MinBudgetTokens)
			}
			if c.MaxBudgetTokens != nil {
				value.MaxBudgetTokens = new(*c.MaxBudgetTokens)
			}
			if c.DefaultBudgetTokens != nil {
				value.DefaultBudgetTokens = new(*c.DefaultBudgetTokens)
			}
			model.Thinking[i] = &value
		case *types.EffortCapability:
			value := *c
			value.Efforts = slices.Clone(c.Efforts)
			value.Summaries = slices.Clone(c.Summaries)
			if c.DefaultEffort != nil {
				value.DefaultEffort = new(*c.DefaultEffort)
			}
			if c.DefaultSummary != nil {
				value.DefaultSummary = new(*c.DefaultSummary)
			}
			model.Thinking[i] = &value
		case *types.DisabledCapability:
			model.Thinking[i] = &types.DisabledCapability{}
		}
	}
}

// checkPageTopics refuses topics outside a page state’s declared scope.
func checkPageTopics(id plugin.ID, page *plugin.Page) error {
	for _, scope := range []plugin.Scope{plugin.ScopeUser, plugin.ScopeConversation} {
		state := page.State(scope)
		if state == nil {
			continue
		}
		for _, topic := range state.Topics {
			if topic.Scope() != scope {
				scopeName := "User"
				if scope == plugin.ScopeConversation {
					scopeName = "Conversation"
				}
				topicName := "Exposes"
				if topic == plugin.TopicJobs {
					topicName = "Jobs"
				}
				return fmt.Errorf(
					"plugin \"%s\"'s %s state follows %s, a topic of another scope",
					id,
					scopeName,
					topicName,
				)
			}
		}
	}
	return nil
}

func (r *Registry) checkPanelKinds(manifest plugin.Manifest) error {
	if manifest.Page == nil {
		return nil
	}
	seen := map[string]bool{}
	for _, kind := range manifest.Page.PanelKinds {
		_, found := r.kindOwner(kind)
		if found || seen[kind] {
			return fmt.Errorf(`plugin "%s" declares the panel kind "%s", which is taken`, manifest.ID, kind)
		}
		seen[kind] = true
	}
	return nil
}
