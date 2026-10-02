package opaque

import "encoding/json"

// +demi:root
// +demi:msgpack
type ManifestMessage struct {
	Manifest json.RawMessage `json:"manifest"`
}
