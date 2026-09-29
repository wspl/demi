package webapi

import (
	"github.com/wspl/demi/go/builtinproto"
)

// `GET …/browser/tabs`: the browser's tabs; a browser that does not run
// has none.
//
//demi:wire open
type BrowserTabs struct {
	Tabs []builtinproto.BrowserTab `json:"tabs" check:"each(func=builtinproto.ValidateBrowserTab)"`
}

// `POST …/browser/tabs { url? }`: a new tab, at `about:blank` without a
// URL.
//
//demi:wire
type OpenTab struct {
	URL **string `json:"url,omitzero" check:"nullable,chars=1..4096"`
}

// `POST …/browser/tabs/:tab/navigate { url }`.
//
//demi:wire
type NavigateTab struct {
	URL string `json:"url" check:"chars=1..4096"`
}

// Where `POST …/browser/tabs/:tab/history` moves a tab.
//
//demi:enum
//demi:export
type HistoryAction string

const (
	HistoryActionBack    HistoryAction = "back"
	HistoryActionForward HistoryAction = "forward"
	HistoryActionReload  HistoryAction = "reload"
)

// `POST …/browser/tabs/:tab/history { action }`.
//
//demi:wire
type TabHistory struct {
	Action HistoryAction `json:"action"`
}
