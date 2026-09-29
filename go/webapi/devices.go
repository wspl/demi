package webapi

import (
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/runnerproto"
)

// How a device came to be: `user` for one its user paired, `managed` for
// the user's Cloud.
//
//demi:enum
//demi:export
type DeviceKind string

const (
	DeviceKindUser    DeviceKind = "user"
	DeviceKindManaged DeviceKind = "managed"
)

// A device as the browser sees it. `platform` is the one its runner
// reported; `online` says whether its runner is connected now; `home` is
// the home directory it reported when it last connected, null until then
// (the backend keeps it in memory only).
//
//demi:wire open
type DeviceDTO struct {
	ID         DeviceID                   `json:"id" check:"func=Validate"`
	Kind       DeviceKind                 `json:"kind"`
	Name       string                     `json:"name"`
	Platform   runnerproto.RunnerPlatform `json:"platform" check:"func=runnerproto.Validate"`
	ClaimedAt  core.Timestamp             `json:"claimedAt" check:"func=core.Validate"`
	LastSeenAt *core.Timestamp            `json:"lastSeenAt" check:"nullable,func=core.Validate"`
	Online     bool                       `json:"online"`
	Home       *string                    `json:"home" check:"nullable"`
}

// `GET /devices`: the caller's paired devices, oldest first.
//
//demi:wire open
type Devices struct {
	Devices []DeviceDTO `json:"devices"`
}

// `POST /devices/claim`: the pairing code a waiting runner printed, spelled
// in any case, with any spaces and dashes.
//
//demi:wire
type Claim struct {
	Code string `json:"code" check:"chars=1.."`
}

// `{ device }`: the answer of a claim.
//
//demi:wire open
type ClaimedDevice struct {
	Device DeviceDTO `json:"device"`
}

// One line of a Host's log (`runner.md` § Host log).
//
//demi:wire open
type DeviceLogLine struct {
	At     core.Timestamp `json:"at" check:"func=core.Validate"`
	Source string         `json:"source"`
	// The conversation the work belonged to, when it belonged to one.
	ConversationID *string `json:"conversationId,omitzero"`
	Text           string  `json:"text"`
}

// `GET /devices/:id/log`: lines oldest first, and the cursor the next read
// continues from.
//
//demi:wire open
type DeviceLog struct {
	Lines []DeviceLogLine `json:"lines"`
	Next  uint64          `json:"next" check:"range=..9007199254740991"`
}
