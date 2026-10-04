package webapiproto

import (
	"github.com/wspl/demi/internal/types"
)

// The most characters of an attached host's name.
const HostNameMax = 64

// A device attached to a conversation.
// +demi:tolerant
type AttachedHost struct {
	DeviceID DeviceID `json:"deviceId"`
	// What the model and the user call the host; unique within the
	// conversation.
	Name string `json:"name"`
	// Where the last shell on the host ended, where the next one starts;
	// null until one ran.
	// +demi:nullable
	Cwd *string `json:"cwd"`
	// Whether the device's runner is connected.
	Online     bool            `json:"online"`
	AttachedAt types.Timestamp `json:"attachedAt"`
}

// `{ hosts }`: the conversation's attached hosts, first attached first.
// +demi:root direction=receive output=web
// +demi:tolerant
type AttachedHosts struct {
	Hosts []AttachedHost `json:"hosts"`
}

// `POST /conversations/:id/hosts { deviceId }`: a device of the user's to
// attach.
// +demi:root direction=send output=web
type AttachHost struct {
	DeviceID DeviceID `json:"deviceId"`
}

// `PATCH /conversations/:id/hosts/:deviceId { name }`: 1 to 64 characters
// after trimming.
// +demi:root direction=send output=web
type RenameHost struct {
	// +demi:length chars min=1 max=64
	Name Trimmed `json:"name"`
}
