package invalid

import "encoding/json"

// +demi:msgpack
type Broken struct {
	Value json.RawMessage `json:"value"`
}
