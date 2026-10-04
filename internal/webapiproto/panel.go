package webapiproto

import "encoding/json"

// The most bytes of one work panel's tabs, as the backend stores them.
const (
	PanelBytesMax = 64 * 1024
	// The most tabs one work panel holds.
	PanelTabsMax = 64
	// The most characters of a tab's id.
	PanelTabIDMax = 64
)

// `GET /conversations/:id/panel`: the tabs in order, and how many changes
// made them.
// +demi:root direction=receive output=web
type WorkPanel struct {
	// +demi:range max=9007199254740991
	Revision uint64     `json:"revision"`
	Tabs     []PanelTab `json:"tabs"`
}

// One tab of the panel: its kind, and what the page and the kind's plugin
// keep for it.
type PanelTab struct {
	// +demi:length chars min=1 max=64
	ID string `json:"id"`
	// +demi:length chars min=1
	Kind string `json:"kind"`
	// +demi:object
	Data json.RawMessage `json:"data"`
}

// `POST /conversations/:id/panel/tabs`: a new tab at `index`, after the
// others without one.
// +demi:root direction=send output=web
type CreatePanelTab struct {
	// +demi:length chars min=1 max=64
	ID string `json:"id"`
	// +demi:length chars min=1
	Kind string `json:"kind"`
	// +demi:object
	Data json.RawMessage `json:"data"`
	// +demi:range max=64
	// +demi:nullable
	Index *uint64 `json:"index,omitempty"`
}

// `PATCH /conversations/:id/panel/tabs/:tab`: the fields of the tab's
// `data` to set, a null one to remove.
// +demi:root direction=send output=web
type UpdatePanelTab struct {
	// +demi:object
	Data json.RawMessage `json:"data"`
}

// `POST /conversations/:id/panel/tabs/:tab/move`: the tab's new place
// among the others.
// +demi:root direction=send output=web
type MovePanelTab struct {
	// +demi:range max=64
	Index uint64 `json:"index"`
}

// What every change of the panel answers: its revision once the change is
// in it.
// +demi:root direction=receive output=web
type PanelRevision struct {
	// +demi:range max=9007199254740991
	Revision uint64 `json:"revision"`
}

// EmptyWorkPanel returns the panel of a conversation that never changed one.
func EmptyWorkPanel() WorkPanel {
	return WorkPanel{Tabs: []PanelTab{}}
}
