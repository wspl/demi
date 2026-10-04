package invalid

import "encoding/json"

var _ json.RawMessage

// +demi:root
type Broken struct {
	// +demi:object yes
	Data json.RawMessage `json:"data"`
}
