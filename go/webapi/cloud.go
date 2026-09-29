package webapi

import ()

// Where the user's Cloud is in its lifecycle (`managed-hosts.md`
// § Lifecycle and capacity): `unallocated` until its first use makes it,
// `off` while no sandbox runs, `booting` until its runner connects,
// `running`, `saving` while it stops, and `resetting` while a reset holds
// it.
//
//demi:enum
//demi:export
type CloudState string

const (
	CloudStateUnallocated CloudState = "unallocated"
	CloudStateOff         CloudState = "off"
	CloudStateBooting     CloudState = "booting"
	CloudStateRunning     CloudState = "running"
	CloudStateSaving      CloudState = "saving"
	CloudStateResetting   CloudState = "resetting"
)

// A reset's phase (`managed-hosts.md` § System reset): it stops the Cloud,
// saves its disks, rebuilds the system on the base the reset selected and
// boots, and ends `ready` or `failed`.
//
//demi:enum
//demi:export
type ResetPhase string

const (
	ResetPhaseStopping   ResetPhase = "stopping"
	ResetPhaseSaving     ResetPhase = "saving"
	ResetPhaseRebuilding ResetPhase = "rebuilding"
	ResetPhaseBooting    ResetPhase = "booting"
	ResetPhaseReady      ResetPhase = "ready"
	ResetPhaseFailed     ResetPhase = "failed"
)

// The Cloud's latest reset: the operation the page named, its phase, and
// why it failed.
//
//demi:wire open
type CloudOperation struct {
	ID    OperationID `json:"id" check:"func=Validate"`
	Phase ResetPhase  `json:"phase"`
	Error *string     `json:"error" check:"nullable"`
}

// The Cloud's device: its identity, which wake, stop and reset keep.
//
//demi:wire open
type CloudDevice struct {
	ID   DeviceID `json:"id" check:"func=Validate"`
	Name string   `json:"name"`
}

// The capacities of a Cloud's two writable filesystems in bytes, or the
// most each may grow to: the capacity, not what is used.
//
//demi:wire open
type CloudVolumes struct {
	SystemBytes uint64 `json:"systemBytes" check:"range=..9007199254740991"`
	HomeBytes   uint64 `json:"homeBytes" check:"range=..9007199254740991"`
}

// `GET /cloud`, and the `cloud` of the product state: the device, its
// lifecycle state, its latest reset, why its last boot, save or reset
// failed, the capacities of its filesystems once its first boot made them,
// and the most they may grow to. Reading it never wakes the Cloud.
//
//demi:wire open
type CloudStatus struct {
	Device    *CloudDevice    `json:"device" check:"nullable"`
	State     CloudState      `json:"state"`
	Operation *CloudOperation `json:"operation" check:"nullable"`
	Error     *string         `json:"error" check:"nullable"`
	Volumes   *CloudVolumes   `json:"volumes" check:"nullable"`
	Limits    CloudVolumes    `json:"limits"`
}

// `POST /cloud/reset`: the operation the page names the reset by; a retry
// with the same id is the same reset.
//
//demi:wire
type CloudReset struct {
	OperationID OperationID `json:"operationId" check:"func=Validate"`
}

// `{ operation }`: the reset a `POST /cloud/reset` admitted, or the one it
// names.
//
//demi:wire open
type CloudResetAnswer struct {
	Operation CloudOperation `json:"operation"`
}
