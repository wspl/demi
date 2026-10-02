package webapi

import (
	"fmt"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/runnerwire"
)

// How a device came to be: `user` for one its user paired, `managed` for
// the user's Cloud.
// +demi:enum user managed
type DeviceKind string

// Values of the preceding enumeration.
const (
	DeviceKindUser    DeviceKind = "user"
	DeviceKindManaged DeviceKind = "managed"
)

// A device as the web app sees it. `platform` is the one its runner
// reported; `online` says whether its runner is connected now; `home` is
// the home directory it reported when it last connected, null until then
// (the backend keeps it in memory only); `installs` are the command
// packages its runner is installing now, as it last reported them
// (`native-runtime.md` § Installation progress).
// +demi:tolerant
type DeviceDTO struct {
	ID        DeviceID                  `json:"id"`
	Kind      DeviceKind                `json:"kind"`
	Name      string                    `json:"name"`
	Platform  runnerwire.RunnerPlatform `json:"platform"`
	ClaimedAt core.Timestamp            `json:"claimedAt"`
	// +demi:nullable
	LastSeenAt *core.Timestamp `json:"lastSeenAt"`
	Online     bool            `json:"online"`
	// +demi:nullable
	Home     *string              `json:"home"`
	Installs []runnerwire.Install `json:"installs"`
}

// `GET /devices`: the caller's paired devices, oldest first.
// +demi:root direction=receive output=web
// +demi:tolerant
type Devices struct {
	Devices []DeviceDTO `json:"devices"`
}

// `POST /devices/claim`: the pairing code a waiting runner printed, spelled
// in any case, with any spaces and dashes.
// +demi:root direction=send output=web
type Claim struct {
	// +demi:length chars min=1
	Code string `json:"code"`
}

// `{ device }`: the answer of a claim.
// +demi:root direction=receive output=web
// +demi:tolerant
type ClaimedDevice struct {
	Device DeviceDTO `json:"device"`
}

// One line of a Host's log (`runner.md` § Host log).
// +demi:tolerant
type DeviceLogLine struct {
	At     core.Timestamp `json:"at"`
	Source string         `json:"source"`
	// The conversation the work belonged to, when it belonged to one.
	ConversationID *string `json:"conversationId,omitempty"`
	Text           string  `json:"text"`
}

// `GET /devices/:id/log`: lines oldest first, and the cursor the next read
// continues from.
// +demi:root direction=receive output=web
// +demi:tolerant
type DeviceLog struct {
	Lines []DeviceLogLine `json:"lines"`
	// +demi:range max=9007199254740991
	Next uint64 `json:"next"`
}

// `?since=&limit=&source=` of the device log. Queries are the backend's
// alone and are not emitted.
// +demi:root
// +demi:tolerant
type DeviceLogQuery struct {
	// The `next` of an earlier answer; without it the answer ends at the
	// newest line.
	// +demi:nullable
	Since *uint64  `json:"since,omitempty"`
	Limit LogLimit `json:"limit"`
	// +demi:nullable
	Source *LogSource `json:"source,omitempty"`
}

// The one source a device log read keeps, such as `runner`.
// +demi:root
// +demi:id
// +demi:length chars min=1
type LogSource string

// How many lines a device log read returns: 1 to what one runner read
// returns, 200 when omitted.
// +demi:root
// +demi:check validateLogLimit
type LogLimit uint64

// DefaultLogLimit is the number of lines returned when the query omits limit.
const DefaultLogLimit LogLimit = 200

func validateLogLimit(limit LogLimit) error {
	if limit < 1 || limit > runnerwire.LogReadLines {
		return fmt.Errorf("limit must be 1 to %d", runnerwire.LogReadLines)
	}
	return nil
}
