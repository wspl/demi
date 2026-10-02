package imported

import "github.com/wspl/demi/internal/commandwire"

// +demi:root
// +demi:msgpack
type Location struct {
	Location *commandwire.ArtifactLocation `json:"location,omitempty"`
}
