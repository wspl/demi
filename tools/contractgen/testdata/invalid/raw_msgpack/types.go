package invalid

import "encoding/json"

// +demi:msgpack
// +demi:root direction=receive
type Broken struct {
	Value json.RawMessage `json:"value"`
}
