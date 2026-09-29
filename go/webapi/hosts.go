package webapi

import (
	"github.com/wspl/demi/go/core"
)

// A device attached to a conversation.
//
//demi:wire open
type AttachedHost struct {
	DeviceID DeviceID `json:"deviceId" check:"func=Validate"`
	// What the model and the user call the host; unique within the
	// conversation.
	Name string `json:"name"`
	// Where the last shell on the host ended, where the next one starts;
	// null until one ran.
	Cwd *string `json:"cwd" check:"nullable"`
	// Whether the device's runner is connected.
	Online     bool           `json:"online"`
	AttachedAt core.Timestamp `json:"attachedAt" check:"func=core.Validate"`
}

// `{ hosts }`: the conversation's attached hosts, first attached first.
//
//demi:wire open
type AttachedHosts struct {
	Hosts []AttachedHost `json:"hosts"`
}

// `POST /conversations/:id/hosts { deviceId }`: a device of the user's to
// attach.
//
//demi:wire
type AttachHost struct {
	DeviceID DeviceID `json:"deviceId" check:"func=Validate"`
}

// `PATCH /conversations/:id/hosts/:deviceId { name }`: 1 to 64 characters
// after trimming.
//
//demi:wire
type RenameHost struct {
	Name Trimmed `json:"name" check:"chars=1..64,func=Validate"`
}
