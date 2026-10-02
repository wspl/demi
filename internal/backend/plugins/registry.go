package plugins

//revive:disable:unused-parameter
// API checkpoint: bodies follow after the API is merged.

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
	"github.com/wspl/demi/internal/plugin"
)

// Registry holds the backend's plugins in registration order, shared by users.
// Construct it with NewRegistry; its published declarations are immutable.
type Registry struct{}

// NewRegistry checks factories in order. Contributions bound to operations that
// serves refuses are left out and logged; an invalid manifest refuses startup.
func NewRegistry(factories []plugin.Factory, serves func(declare.NativeOperation) bool) (*Registry, error) {
	panic("not written: b-plugins")
}

// ContextSources returns the context sources in registration order.
func (r *Registry) ContextSources() []plugin.ID { panic("not written: b-plugins") }

// Profiles returns the plugins' profiles in registration order.
func (r *Registry) Profiles() []core.Profile { panic("not written: b-plugins") }

// Streams returns every user stream the startup catalog serves, in registration order.
func (r *Registry) Streams() []plugin.Stream { panic("not written: b-plugins") }

// Followers returns the plugins whose page state follows topic, in registration order.
func (r *Registry) Followers(topic plugin.Topic) []plugin.ID { panic("not written: b-plugins") }
