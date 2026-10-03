// Package pagemeta defines the manifest printer's interface to contractgen.
package pagemeta

import "encoding/json"

// Page is tool-internal metadata, not a Demi wire contract. Both tools use
// encoding/json; this interface deliberately has no generated contract code.
type Page struct {
	ID        string     `json:"id"`
	Package   string     `json:"package"`
	Schemas   []Schema   `json:"schemas"`
	Constants []Constant `json:"constants"`
}

// Schema carries a manifest schema and its use from the page's perspective.
type Schema struct {
	Direction string          `json:"direction"`
	Value     json.RawMessage `json:"value"`
}

// Constant carries a stream's shared constant without reordering its JSON value.
type Constant struct {
	Name        string          `json:"name"`
	Description string          `json:"description"`
	Value       json.RawMessage `json:"value"`
}
