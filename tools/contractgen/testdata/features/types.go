// Package features exercises contract capabilities independently of the corpora.
package features

import "encoding/json"
import "github.com/wspl/demi/tools/contractgen/testdata/blocks"

//go:generate go run ../..

// +demi:id pattern=^id_[a-z]+$
type Identifier string

// +demi:base64
type Encoded string

// +demi:root direction=receive output=plugin-test
// +demi:tolerant
type Tree struct {
	Name     Identifier `json:"name"`
	Children []Tree     `json:"children"`
	Parent   *Tree      `json:"parent,omitzero"`
}

// +demi:root direction=send output=web
// +demi:strict
type Request struct {
	ID   Identifier                 `json:"id"`
	Body contracts.UserContentBlock `json:"body"`
	Data Encoded                    `json:"data"`
	// +demi:nullable
	Label *string         `json:"label"`
	Extra json.RawMessage `json:"extra"`
}

// +demi:strict
type Common struct {
	ID Identifier `json:"id"`
}

// +demi:root direction=send output=web
type Flat struct {
	Common
	Text string `json:"text"`
}

type Page[T any] struct {
	// +demi:length min=1
	Items []T `json:"items"`
}

// +demi:root direction=receive output=plugin-test
type Names Page[Identifier]

// +demi:msgpack
// +demi:root
type Records struct {
	Values map[string]*string `json:"values"`
	// +demi:timestamp
	At string `json:"at"`
}
