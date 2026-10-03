//revive:disable:exported
// Contract names keep the golden manifest's schema titles and generated TypeScript exports.

package pluginbrowser

import "github.com/wspl/demi/internal/cmdpkg/browser/browserop"

//go:generate go run ../..

// The most characters of a URL a method takes.
const URLMax = 4096

// The conversation state: the conversation browser's tabs, none while it
// does not run.
// +demi:root direction=receive output=plugin-browser
// +demi:schema
// +demi:tolerant
type BrowserTabs struct {
	Tabs []browserop.BrowserTab `json:"tabs"`
}

// `open { url? }`: a new tab, at `about:blank` without a URL.
// +demi:root direction=send output=plugin-browser
// +demi:schema
type OpenTab struct {
	// +demi:nullable
	// +demi:length chars min=1 max=4096
	URL *string `json:"url,omitempty"`
}

// What `open` answers.
// +demi:root direction=receive output=plugin-browser
// +demi:schema
// +demi:tolerant
type OpenedTab struct {
	Tab browserop.BrowserTab `json:"tab"`
}

// `close { tab }`.
// +demi:root direction=send output=plugin-browser
// +demi:schema
type CloseTab struct {
	Tab string `json:"tab"`
}

// `navigate { tab, url }`.
// +demi:root direction=send output=plugin-browser
// +demi:schema
type NavigateTab struct {
	Tab string `json:"tab"`
	// +demi:length chars min=1 max=4096
	URL string `json:"url"`
}

// Where `history` moves a tab.
// +demi:root direction=send output=plugin-browser
// +demi:enum back forward reload
type HistoryAction string

const (
	// HistoryBack moves to the preceding history entry.
	HistoryBack HistoryAction = "back"
	// HistoryForward moves to the following history entry.
	HistoryForward HistoryAction = "forward"
	// HistoryReload reloads the current entry.
	HistoryReload HistoryAction = "reload"
)

// `history { tab, action }`.
// +demi:root direction=send output=plugin-browser
// +demi:schema
type TabHistory struct {
	Tab    string        `json:"tab"`
	Action HistoryAction `json:"action"`
}
