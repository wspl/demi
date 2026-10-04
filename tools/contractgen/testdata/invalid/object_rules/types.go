package invalid

import "encoding/json"

var _ json.RawMessage

// +demi:root
type Broken struct {
	// +demi:object
	// +demi:length min=1
	Data json.RawMessage `json:"data"`
}
