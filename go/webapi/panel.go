package webapi

import (
	"encoding/json/jsontext"
)

// `GET/PUT /conversations/:id/panel`: what the panel selects, `"change"`,
// `"file"` or a tab's id, and its tabs in the user's order.
//
//demi:wire
type WorkPanel struct {
	Selection string     `json:"selection" check:"bytes=1.."`
	Tabs      []PanelTab `json:"tabs" check:"items=..64"`
}

// One tab of the panel: what the page keeps for it, which only the page
// reads.
//
//demi:wire
type PanelTab struct {
	ID   string         `json:"id" check:"bytes=1.."`
	Kind string         `json:"kind" check:"bytes=1.."`
	Data jsontext.Value `json:"data"`
}
