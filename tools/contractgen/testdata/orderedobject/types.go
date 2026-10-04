package orderedobject

import "encoding/json"

// +demi:root direction=send output=protocol
// +demi:schema
type Input struct {
	// +demi:object
	Data json.RawMessage `json:"data"`
}

// +demi:root
type Envelope struct {
	Input Input `json:"input"`
}

// +demi:root
type Float32 float32

// +demi:root
type FloatValues struct {
	Values []Float32 `json:"values"`
}
