package expose

import (
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

//go:generate go run github.com/wspl/demi/tools/contractgen

//revive:disable:exported
// Contract descriptions are verbatim Rust product text.

// The input of `demi expose add`.
// +demi:root
// +demi:schema
// +demi:strict
type AddArgs struct {
	// host:port, or a bare port meaning 127.0.0.1
	Address string `json:"address"`
	// Host name or device id from demi host list; the main host by default
	Host *string `json:"host,omitempty"`
}

// The input of `demi expose list`.
// +demi:root
// +demi:schema
// +demi:strict
type ListArgs struct{}

// The input of `demi expose renew` and `remove`.
// +demi:root
// +demi:schema
// +demi:strict
type NumberArgs struct {
	// Expose number, as add and list print it
	Number uint64 `json:"number"`
}

// An expose as the commands print it: by its number, never by its id,
// which is the URL's credential.
// +demi:root
// +demi:schema
// +demi:tolerant
type ExposeLine struct {
	Number uint64 `json:"number"`
	// The device's name.
	Device    string         `json:"device"`
	Address   string         `json:"address"`
	URL       string         `json:"url"`
	ExpiresAt core.Timestamp `json:"expiresAt"`
}

// `{ expose }`: what `add` and `renew` print with `--json`.
// +demi:root
// +demi:schema
// +demi:tolerant
type ExposeAnswer struct {
	Expose ExposeLine `json:"expose"`
}

// `{ exposes }`: what `list` prints with `--json`.
// +demi:root
// +demi:schema
// +demi:tolerant
type ExposeLines struct {
	Exposes []ExposeLine `json:"exposes"`
}

// The plugin's state for the user's pages.
// +demi:root direction=receive output=plugin-expose
// +demi:schema
// +demi:tolerant
type ExposeState struct {
	// Whether the instance has an expose domain; without one the product
	// shows no expose controls.
	Available bool `json:"available"`
	// Every live expose of the user's, soonest expiry first.
	Exposes []ExposeEntry `json:"exposes"`
}

// An expose as the menu shows it, with the name of the device it is on.
// +demi:tolerant
type ExposeEntry struct {
	ID webapi.ExposeID `json:"id"`
	// +demi:range min=1 max=9007199254740991 schema-only
	Number   uint64          `json:"number"`
	DeviceID webapi.DeviceID `json:"deviceId"`
	// The device's name, the Cloud's as `Cloud`.
	DeviceName string               `json:"deviceName"`
	Address    webapi.ExposeAddress `json:"address"`
	URL        string               `json:"url"`
	ExpiresAt  core.Timestamp       `json:"expiresAt"`
}

// `renew { expose }` and `remove { expose }`.
// +demi:root direction=send output=plugin-expose
// +demi:schema
// +demi:strict
type ExposeCall struct {
	// The expose's id.
	Expose string `json:"expose"`
}

// +demi:root
// +demi:strict
type numbers struct {
	// The number the next expose takes; it only grows.
	Next uint64 `json:"next"`
	// Each live expose's number, by its id.
	Exposes map[string]uint64 `json:"exposes"`
}
