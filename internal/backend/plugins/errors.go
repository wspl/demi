package plugins

import (
	"errors"
	"fmt"
)

// ErrUnknownPlugin means no registered plugin has the requested id (or, for a
// conversation state, no plugin with that id has a conversation page).
//
//nolint:staticcheck // Product text, shown to the user as it is.
var ErrUnknownPlugin = errors.New("No plugin")

// unknownPlugin returns ErrUnknownPlugin naming id, as `No plugin "id"`.
func unknownPlugin(id string) error {
	return fmt.Errorf("%w \"%s\"", ErrUnknownPlugin, id)
}

// PageCallErrorKind identifies why a page call did not answer.
type PageCallErrorKind uint8

const (
	// Disabled means the user has the plugin off.
	Disabled PageCallErrorKind = iota
	// UnknownMethod means the plugin declares no such method in this scope.
	UnknownMethod
	// InvalidParams means the parameters do not fit the method's schema.
	InvalidParams
	// PluginFailed means the instance failed to answer the request.
	PluginFailed
)

// PageCallError describes a page call or conversation-state refusal.
type PageCallError struct {
	// Kind selects the refusal.
	Kind PageCallErrorKind
	// Plugin identifies the requested plugin.
	Plugin string
	// Method identifies the method for UnknownMethod.
	Method string
	// Message holds the parameter validation diagnostic for InvalidParams.
	Message string
	// Err retains the plugin failure for PluginFailed.
	Err error
}

// Error returns the page diagnostic.
func (e *PageCallError) Error() string {
	switch e.Kind {
	case Disabled:
		return fmt.Sprintf("The plugin \"%s\" is off", e.Plugin)
	case UnknownMethod:
		return fmt.Sprintf("The plugin \"%s\" has no method \"%s\" here", e.Plugin, e.Method)
	case InvalidParams:
		return e.Message
	case PluginFailed:
		return e.Err.Error()
	}
	return "unknown plugin page error"
}

// Unwrap returns the underlying plugin failure, when present.
func (e *PageCallError) Unwrap() error { return e.Err }
