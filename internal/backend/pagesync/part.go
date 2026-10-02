package pagesync

import "github.com/wspl/demi/internal/webapi"

// Kind identifies a part of the product state, in delivery order.
type Kind uint8

// Parts are delivered in this order so summaries precede conversation order.
const (
	Conversation Kind = iota
	ConversationOrder
	Preferences
	User
	Workspaces
	Devices
	Providers
	Cloud
	Plugins
	Plugin
)

// Part is a part of the product state that a page shows. ConversationID is
// set only for Conversation; PluginID is set only for Plugin. Callers leave
// fields that do not belong to the selected kind zero.
// These are internal change marks, not wire values.
type Part struct {
	Kind           Kind
	ConversationID webapi.ConversationID
	PluginID       string
}
