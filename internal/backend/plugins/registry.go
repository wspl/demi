package plugins

import (
	"fmt"
	"log/slog"
	"slices"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
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
func NewRegistry(factories []plugin.Factory, serves func(declare.NativeOperation) bool) (*Registry, error) {
	r := &Registry{}
	ids, profiles, streams, pages := map[string]bool{}, map[string]bool{}, map[string]bool{}, map[string]bool{}
	for _, factory := range factories {
		manifest := factory.Manifest()
		id := string(manifest.ID)
		if ids[id] {
			return nil, &RegistryError{Kind: DuplicateID, Plugin: manifest.ID}
		}
		ids[id] = true
		for _, profile := range manifest.Profiles {
			reason := ""
			if profile.Name == core.ProfileInherit {
				reason = "is reserved for inheriting the parent"
			} else if profiles[profile.Name] {
				reason = "another plugin declares"
			}
			if reason != "" {
				return nil, &RegistryError{Kind: InvalidProfile, Plugin: manifest.ID, Name: profile.Name, Reason: reason}
			}
			profiles[profile.Name] = true
		}
		for _, stream := range manifest.Streams {
			if streams[stream.Name] {
				return nil, &RegistryError{Kind: TakenStream, Plugin: manifest.ID, Name: stream.Name}
			}
			streams[stream.Name] = true
		}
		if page := manifest.Page; page != nil {
			if pages[page.Package] {
				return nil, &RegistryError{Kind: TakenPagePackage, Plugin: manifest.ID, Name: page.Package}
			}
			pages[page.Package] = true
			for _, scope := range []plugin.Scope{plugin.ScopeUser, plugin.ScopeConversation} {
				if state := page.State(scope); state != nil {
					for _, topic := range state.Topics {
						if topic.Scope() != scope {
							return nil, &RegistryError{Kind: ForeignTopic, Plugin: manifest.ID, Scope: scope, Topic: topic}
						}
					}
				}
			}
		}
		// The generated codec detaches the manifest and checks values Rust's types
		// make unrepresentable (notably the reserved execution id).
		data, err := manifest.MarshalJSON()
		if err != nil {
			return nil, &RegistryError{Kind: RefusedCommands, Plugin: manifest.ID, Err: err}
		}
		detached, err := plugin.DecodeManifest(data)
		if err != nil {
			return nil, &RegistryError{Kind: RefusedCommands, Plugin: manifest.ID, Err: err}
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
func (r *Registry) Profiles() []core.Profile {
	profiles := []core.Profile{}
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
func served(factory plugin.Factory, manifest plugin.Manifest, serves func(declare.NativeOperation) bool) registered {
	packages := map[string]bool{}
	keep := func(what, name string, operations []declare.NativeOperation) bool {
		for _, op := range operations {
			if !serves(op) {
				slog.Info("a part whose package the catalog does not serve is left out", "plugin", manifest.ID, "what", what, "name", name)
				return false
			}
		}
		for _, op := range operations {
			packages[op.Package] = true
		}
		return true
	}
	manifest.Commands = slices.DeleteFunc(manifest.Commands, func(c plugin.Commands) bool {
		return !keep("command", declare.Name(c.Tree.Node), nativeOperations(c.Tree.Node))
	})
	manifest.Streams = slices.DeleteFunc(manifest.Streams, func(s plugin.Stream) bool {
		return !keep("user stream", s.Name, []declare.NativeOperation{s.Operation})
	})
	if page := manifest.Page; page != nil {
		if page.User != nil && !keep("page state", "user", page.User.Operations) {
			page.User = nil
		}
		if page.Conversation != nil && !keep("page state", "conversation", page.Conversation.Operations) {
			page.Conversation = nil
		}
		page.Methods = slices.DeleteFunc(page.Methods, func(m plugin.Method) bool { return !keep("page method", m.Name, m.Operations) })
	}
	names := make([]string, 0, len(packages))
	for name := range packages {
		names = append(names, name)
	}
	slices.Sort(names)
	return registered{factory: factory, manifest: manifest, packages: names}
}

// nativeOperations collects the package operations of one plugin command tree.
func nativeOperations(tree declare.Node[declare.NativeOperation]) []declare.NativeOperation {
	var operations []declare.NativeOperation
	switch n := tree.(type) {
	case *declare.Leaf[declare.NativeOperation]:
		switch kind := n.Kind.(type) {
		case *declare.Native[declare.NativeOperation]:
			operations = append(operations, kind.Binding)
		case *declare.RPC[declare.NativeOperation]:
		}
	case *declare.Group[declare.NativeOperation]:
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
			name := declare.Name(c.Tree.Node)
			strip := 0
			if c.Placement == plugin.PlacementDemi {
				if names[name] {
					return nil, &RegistryError{Kind: TakenCommand, Plugin: p.manifest.ID, Name: "demi " + name}
				}
				names[name] = true
				strip = 1
			} else if name == plugin.DemiRoot {
				return nil, &RegistryError{Kind: TakenCommand, Plugin: p.manifest.ID, Name: name}
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
				return nil, &RegistryError{Kind: RefusedCommands, Plugin: p.manifest.ID, Err: err}
			}
		}
	}
	return set, nil
}

// copyProfiles detaches mutable profile fields returned to a conversation tree.
func copyProfiles(profiles []core.Profile) []core.Profile {
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
func copyProfileModel(selection core.ModelSelection) *core.ModelSelection {
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
	model.Thinking = slices.Clone(model.Thinking)
	for i, capability := range model.Thinking {
		switch c := capability.(type) {
		case *core.AdaptiveCapability:
			value := *c
			value.Efforts = slices.Clone(c.Efforts)
			if c.DefaultEffort != nil {
				value.DefaultEffort = new(*c.DefaultEffort)
			}
			model.Thinking[i] = &value
		case *core.BudgetCapability:
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
		case *core.EffortCapability:
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
		case *core.DisabledCapability:
			model.Thinking[i] = &core.DisabledCapability{}
		}
	}
	switch config := selection.Thinking.(type) {
	case *core.AdaptiveConfig:
		selection.Thinking = new(*config)
	case *core.BudgetConfig:
		selection.Thinking = new(*config)
	case *core.EffortConfig:
		value := *config
		if value.Summary != nil {
			value.Summary = new(*value.Summary)
		}
		selection.Thinking = &value
	case *core.DisabledConfig:
		selection.Thinking = &core.DisabledConfig{}
	}
	return &selection
}
