// Package presence exercises nullable optional fields at both wire boundaries.
package presence

import "github.com/wspl/demi/internal/runnerwire"

//go:generate go run ../..

// +demi:root direction=send output=plugin-presence
// +demi:msgpack
// +demi:schema
type Patch struct {
	// +demi:nullable
	// +demi:length max=4
	Option *string `json:"option,omitempty"`
	// +demi:nullable
	// +demi:length max=4
	Double **string `json:"double,omitempty"`
	// +demi:nullable
	Items **[]string `json:"items,omitempty"`
}

// +demi:root
// +demi:msgpack
type TimestampPatch struct {
	// +demi:nullable
	// +demi:timestamp
	At **string `json:"at,omitempty"`
}

// +demi:root direction=receive output=plugin-presence
// +demi:schema
type InstallEnvelope struct {
	Install runnerwire.Install `json:"install"`
}
