//revive:disable:exported
// Contract names are fixed by the schema titles and generated TypeScript exports.

package browser

import "github.com/wspl/demi/internal/cmdpkg/browser/browserop"

//go:generate go run github.com/wspl/demi/tools/contractgen

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

// `bind { panelTab }`: a browser tab for the panel tab, which Retry and
// Reload ask for.
// +demi:root direction=send output=plugin-browser
// +demi:schema
type BindTab struct {
	PanelTab string `json:"panelTab"`
}

// `sync {}`: the panel's tabs updated from the browser's.
// +demi:root direction=send output=plugin-browser
// +demi:schema
type SyncTabs struct{}

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
