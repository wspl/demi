package plugin

import (
	"encoding/json"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/declare"
)

//revive:disable:exported
// Contract descriptions are verbatim Rust product text.

// A plugin's declarations.
// +demi:root
type Manifest struct {
	ID ID `json:"id"`
	// Its name in settings, such as `Todo list`.
	Name string `json:"name"`
	// What it does, in one sentence, which settings show.
	Description string `json:"description"`
	// Its command groups and roots (`plugins.md` § Commands).
	// +demi:default
	Commands []Commands `json:"commands"`
	// Its subagent profiles (`plugins.md` § Profiles).
	// +demi:default
	Profiles []core.Profile `json:"profiles"`
	// Whether it is a context source, asked before each provider request
	// of a node while its user has it on (`plugins.md` § Prompt text and
	// context).
	// +demi:default
	Context bool `json:"context"`
	// Its user streams (`plugins.md` § Calling its command package).
	// +demi:default
	Streams []Stream `json:"streams"`
	// Its part of the web app's data and calls (`plugins.md` § The page).
	// +demi:nullable
	Page *Page `json:"page,omitempty"`
}

// A user stream: a page connects to `operation` on the conversation's main
// Host for as long as it keeps the stream open. Its messages and constants
// are declared for the page's generated types (`plugin-pages.md` § Types);
// the backend relays the stream's bytes without reading them.
// +demi:root
type Stream struct {
	// The name the page opens it by, taken once among all plugins.
	Name      string                  `json:"name"`
	Operation declare.NativeOperation `json:"operation"`
	// The messages the page receives.
	Receives Schema `json:"receives"`
	// The messages the page sends.
	Sends Schema `json:"sends"`
	// The constants the stream's two ends share, such as its frame kinds.
	// +demi:default
	Constants []Constant `json:"constants"`
}

// A value both ends of a stream use, such as `LIVE_VIDEO_FRAME = 2`.
// +demi:root
type Constant struct {
	// An upper-case TypeScript name.
	Name        string          `json:"name"`
	Description string          `json:"description"`
	Value       json.RawMessage `json:"value"`
}

// A plugin's page (`plugins.md` § The page): its package, and what it
// reads and calls.
// +demi:root
type Page struct {
	// The page package, such as `@demicodes/plugin-browser`, which the web
	// app's registry lists and the plugin's types are generated into.
	Package string `json:"package"`
	// Its state for the user, which reaches every page on the sync channel.
	// +demi:nullable
	User *State `json:"user,omitempty"`
	// Its state for one conversation, which a page reads by revision.
	// +demi:nullable
	Conversation *State `json:"conversation,omitempty"`
	// +demi:default
	Methods []Method `json:"methods"`
}

// One scope of a page's state.
// +demi:root
type State struct {
	Schema Schema `json:"schema"`
	// The product changes that mark it changed, each of the state's scope.
	// +demi:default
	Topics []Topic `json:"topics"`
	// The operations of a command package a read of it calls. A state one
	// of whose operations the startup catalog does not serve is left out,
	// as a method is.
	// +demi:default
	Operations []declare.NativeOperation `json:"operations"`
}

// A product change a page state can follow (`plugins.md` § Topics).
// +demi:root
// +demi:enum exposes jobs
type Topic string

const (
	// The user's exposes: one is created, renewed or destroyed, or the
	// earliest one expires.
	TopicExposes Topic = "exposes"
	// A job of the conversation ends.
	TopicJobs Topic = "jobs"
)

// A page method: what its parameters and result look like, whom it is
// for, and the package operations it calls.
// +demi:root
type Method struct {
	// A snake_case word, such as `renew`.
	Name   string `json:"name"`
	Scope  Scope  `json:"scope"`
	Params Schema `json:"params"`
	Result Schema `json:"result"`
	// The operations of a command package the method calls. A method one
	// of whose operations the startup catalog does not serve is left out,
	// as a command group is.
	// +demi:default
	Operations []declare.NativeOperation `json:"operations"`
}

// Whom a page method is called for, which decides its route
// (`web-api.md` § Plugin calls), and whom a page state is of.
// +demi:root
// +demi:enum user conversation
type Scope string

const (
	// The user: `POST /api/plugins/:plugin/calls/:method`.
	ScopeUser Scope = "user"
	// One conversation of the user's:
	// `POST /api/conversations/:id/plugins/:plugin/calls/:method`.
	ScopeConversation Scope = "conversation"
)

// One command tree of a plugin, and where it goes in the command set every
// node starts from.
// +demi:root
type Commands struct {
	Placement Placement `json:"placement"`
	// The tree, as data: an `rpc` leaf reaches the plugin as a command
	// request, a native leaf runs its operation on the Host.
	Tree Declaration `json:"tree"`
}

// Where a plugin's command tree goes.
// +demi:root
// +demi:enum demi root
type Placement string

const (
	// A named group of the `demi` root, for a plugin of this repository.
	PlacementDemi Placement = "demi"
	// A root command of its own.
	PlacementRoot Placement = "root"
)

// manifests is the manifest-list contract used by the Rust fidelity corpus.
// +demi:root
type manifests []Manifest
