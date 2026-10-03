package backend

import (
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/plugin"
)

// BuiltinFamilies returns the families built into the backend: anthropic,
// claude-code, codex, google, grok-build and openai. Tests can register
// additional scripted families in the returned registry.
func BuiltinFamilies() *providers.FamilyRegistry { panic("not written: b-backend") }

// BuiltinPlugins returns the plugins built into the backend in registration
// order: file, todo, browser, expose, skills, changes and file-browser.
// Declaration failures are returned instead of panicking.
func BuiltinPlugins() ([]plugin.Factory, error) { panic("not written: b-backend") }
