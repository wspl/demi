package webapiproto

import (
	"encoding/json"
)

// A plugin of the backend, in its order of registration.
// +demi:tolerant
type PluginEntry struct {
	ID   string `json:"id"`
	Name string `json:"name"`
	// What it does, in one sentence.
	Description string `json:"description"`
	// Whether the user has it on.
	Enabled bool `json:"enabled"`
	// The command packages its commands, user streams and page methods
	// bind that the backend's catalog serves, such as `demi.browser`.
	Packages []string `json:"packages"`
}

// `PUT /plugins/:plugin { enabled }`.
// +demi:root direction=send output=web
type PluginSwitch struct {
	Enabled bool `json:"enabled"`
}

// `GET /conversations/:id/plugins/:plugin/state` (`web-api.md`
// § Conversation state of plugins): the plugin's state for the
// conversation, valid against the plugin's declared schema, and the
// revision it was read at.
// +demi:root direction=receive output=web
// +demi:tolerant
type PluginStateAnswer struct {
	// +demi:range max=9007199254740991
	Revision uint64          `json:"revision"`
	State    json.RawMessage `json:"state"`
}
