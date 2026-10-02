package skills

import (
	"context"
	"encoding/json"
	"fmt"
	"slices"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
)

// Resolve maps a source's public URL to the repository a test serves.
type Resolve func(string) string

// Factory creates each user's skills instance and declares its settings page.
type Factory struct {
	manifest plugin.Manifest
	resolve  Resolve
	clock    core.Clock
}

// New constructs the factory for public GitHub and HTTPS repositories.
func New() (*Factory, error) {
	return NewResolving(func(url string) string { return url }, core.SystemClock{})
}

// NewResolving constructs a factory with a test's HTTP repositories and clock.
func NewResolving(resolve Resolve, clock core.Clock) (*Factory, error) {
	state, err := pageSchema(StateJSONSchema())
	if err != nil {
		return nil, err
	}
	page := &plugin.Page{Package: "@demicodes/plugin-skills", User: &plugin.State{Schema: state}}
	for _, declaration := range []struct {
		name           string
		params, result json.RawMessage
	}{
		{"add_source", AddSourceJSONSchema(), AddedSourceJSONSchema()},
		{"update_source", SourceCallJSONSchema(), json.RawMessage(`{"type":"null"}`)},
		{"remove_source", SourceCallJSONSchema(), json.RawMessage(`{"type":"null"}`)},
		{"set_enabled", SetEnabledJSONSchema(), json.RawMessage(`{"type":"null"}`)},
		{"set_source_enabled", SetSourceEnabledJSONSchema(), json.RawMessage(`{"type":"null"}`)},
	} {
		params, err := pageSchema(declaration.params)
		if err != nil {
			return nil, err
		}
		result, err := pageSchema(declaration.result)
		if err != nil {
			return nil, err
		}
		page.Methods = append(page.Methods, plugin.Method{Name: declaration.name, Scope: plugin.ScopeUser, Params: params, Result: result})
	}
	return &Factory{manifest: plugin.Manifest{ID: "skills", Name: "Skills", Description: "Workflows the agent follows: skills from git repositories you add, and those your repository carries.", Context: true, Page: page}, resolve: resolve, clock: clock}, nil
}

// pageSchema compiles a generated skills page contract through its owning codec.
func pageSchema(document json.RawMessage) (plugin.Schema, error) {
	var schema plugin.Schema
	if err := schema.UnmarshalJSON(document); err != nil {
		return schema, fmt.Errorf("skills page schema: %w", err)
	}
	return schema, nil
}

// Manifest returns the complete page and context declarations.
func (f *Factory) Manifest() plugin.Manifest {
	manifest := f.manifest
	page := *manifest.Page
	state := *page.User
	page.User = &state
	page.Methods = slices.Clone(page.Methods)
	manifest.Page = &page
	return manifest
}

// Instance creates an owner whose lifetime ends when the host calls Close.
func (f *Factory) Instance() plugin.Plugin {
	ctx, cancel := context.WithCancel(context.Background())
	return &instance{ctx: ctx, cancel: cancel, resolve: f.resolve, clock: f.clock, fetching: map[string]bool{}, projects: map[projectKey]projectSearch{}, mutations: make(chan struct{}, 1)}
}
