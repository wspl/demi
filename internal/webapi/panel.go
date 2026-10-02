package webapi

import (
	"errors"

	"encoding/json"
)

// The most bytes of one work panel's document, as the backend stores it.
const PanelBytesMax = 64 * 1024

// The most tabs one work panel holds.
const PanelTabsMax = 64

// `GET/PUT /conversations/:id/panel`: what the panel selects, a tab's id
// or a pinned kind's id such as `"change"`, or null, and its tabs in the
// user's order.
// +demi:root direction=receive output=web
// +demi:check validateWorkPanel
type WorkPanel struct {
	// +demi:nullable
	Selection *string `json:"selection"`
	// +demi:length max=64
	Tabs []PanelTab `json:"tabs"`
}

// One tab of the panel: what the page keeps for it, which only the page
// reads.
type PanelTab struct {
	// +demi:length chars min=1
	ID string `json:"id"`
	// +demi:length chars min=1
	Kind string          `json:"kind"`
	Data json.RawMessage `json:"data"`
}

// The panel of a conversation that never saved one.
func EmptyWorkPanel() WorkPanel { return WorkPanel{Tabs: []PanelTab{}} }

func validateWorkPanel(panel WorkPanel) error {
	if panel.Selection != nil && *panel.Selection == "" {
		return errors.New("selection must not be empty")
	}
	return nil
}
