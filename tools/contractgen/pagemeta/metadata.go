// Package pagemeta defines the manifest printer's interface to contractgen.
package pagemeta

import "encoding/json"

// Page is tool-internal metadata, not a Demi wire contract. Both tools use
// encoding/json; this interface deliberately has no generated contract code.
type Page struct {
	// ID names the registered plugin.
	ID string `json:"id"`
	// Package names the workspace package that provides the page.
	Package string `json:"package"`
	// Schemas lists the wire schemas used by the page.
	Schemas []Schema `json:"schemas"`
	// Constants lists the shared stream values exposed to the page.
	Constants []Constant `json:"constants"`
}

// Schema carries a manifest schema and its use from the page's perspective.
type Schema struct {
	// Direction records whether the page receives or sends the value.
	Direction string `json:"direction"`
	// Value holds the schema as JSON.
	Value json.RawMessage `json:"value"`
}

// Constant carries a stream's shared constant without reordering its JSON value.
type Constant struct {
	// Name is the constant name emitted to TypeScript.
	Name string `json:"name"`
	// Description documents the constant in the generated module.
	Description string `json:"description"`
	// Value holds the constant as JSON, preserving its member order.
	Value json.RawMessage `json:"value"`
}
