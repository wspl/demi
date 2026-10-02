package plugins

import "github.com/wspl/demi/internal/plugin"

// RegistryErrorKind identifies a startup manifest refusal.
type RegistryErrorKind uint8

const (
	// DuplicateID means two plugins declare the same id.
	DuplicateID RegistryErrorKind = iota
	// InvalidProfile means a profile name is reserved or duplicated.
	InvalidProfile
	// TakenCommand means a command name is already owned.
	TakenCommand
	// TakenStream means a user stream name is already owned.
	TakenStream
	// TakenPagePackage means another plugin owns the page package.
	TakenPagePackage
	// ForeignTopic means a state follows a topic of another scope.
	ForeignTopic
	// RefusedCommands means the command set refused a declaration.
	RefusedCommands
)

// RegistryError explains why the backend cannot start with its plugins.
// Kind selects the applicable fields; Err retains a refused command's cause.
type RegistryError struct {
	// Kind identifies the manifest rule that failed.
	Kind RegistryErrorKind
	// Plugin identifies the plugin whose manifest failed.
	Plugin plugin.ID
	// Name is the profile, command, stream or page package that failed.
	Name string
	// Reason explains a profile refusal.
	Reason string
	// Scope is the page state's scope for ForeignTopic.
	Scope plugin.Scope
	// Topic is the topic of another scope for ForeignTopic.
	Topic plugin.Topic
	// Err is the underlying command registration failure.
	Err error
}

// Error returns the Rust-compatible startup diagnostic.
func (e *RegistryError) Error() string { panic("not written: b-plugins") }

// Unwrap returns the command registration failure, when present.
func (e *RegistryError) Unwrap() error { panic("not written: b-plugins") }

// PageCallErrorKind identifies why a page call did not answer.
type PageCallErrorKind uint8

const (
	// UnknownPlugin means the route names no plugin or conversation state.
	UnknownPlugin PageCallErrorKind = iota
	// Disabled means the user has the plugin off.
	Disabled
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

// Error returns the Rust-compatible page diagnostic.
func (e *PageCallError) Error() string { panic("not written: b-plugins") }

// Unwrap returns the underlying plugin failure, when present.
func (e *PageCallError) Unwrap() error { panic("not written: b-plugins") }

// SwitchError means a plugin choice could not be changed. A nil Err means
// Plugin is unknown; otherwise Err is the underlying storage failure.
type SwitchError struct {
	// Plugin is the requested plugin id.
	Plugin string
	// Err is the storage failure, or nil for an unknown plugin.
	Err error
}

// Error returns the Rust-compatible switch diagnostic.
func (e *SwitchError) Error() string { panic("not written: b-plugins") }

// Unwrap returns the underlying storage failure, when present.
func (e *SwitchError) Unwrap() error { panic("not written: b-plugins") }
