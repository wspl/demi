package pagesync

import "github.com/wspl/demi/internal/webapiproto"

// Kind identifies a part of the product state, in delivery order.
type Kind uint8

// Parts are delivered in this order so summaries precede conversation order.
const (
	// Conversation marks a conversation summary change.
	Conversation Kind = iota
	// ConversationOrder marks a sidebar order change.
	ConversationOrder
	// Preferences marks a preference change.
	Preferences
	// User marks an account change.
	User
	// Workspaces marks a workspace change.
	Workspaces
	// Devices marks a device change.
	Devices
	// Providers marks a provider change.
	Providers
	// Cloud marks a Cloud state change.
	Cloud
	// Plugins marks the enabled plugin list changing.
	Plugins
	// Plugin marks one plugin’s state changing.
	Plugin
)

// Part is a part of the product state that a page shows. ConversationID is
// set only for Conversation; PluginID is set only for Plugin. Callers leave
// fields that do not belong to the selected kind zero.
// These are internal change marks, not wire values.
type Part struct {
	Kind           Kind
	ConversationID webapiproto.ConversationID
	PluginID       string
}
