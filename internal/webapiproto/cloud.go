package webapiproto

// Where the user's Cloud is in its lifecycle (`managed-hosts.md`
// § Lifecycle and capacity): `unallocated` until its first use makes it,
// `off` while no sandbox runs, `booting` until its runner connects,
// `running`, `saving` while it stops, and `resetting` while a reset holds
// it.
// +demi:enum unallocated off booting running saving resetting
type CloudState string

// Values of the preceding enumeration.
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
// +demi:enum stopping saving rebuilding booting ready failed
type ResetPhase string

// Values of the preceding enumeration.
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
// +demi:tolerant
type CloudOperation struct {
	ID    OperationID `json:"id"`
	Phase ResetPhase  `json:"phase"`
	// +demi:nullable
	Error *string `json:"error"`
}

// The Cloud's device: its identity, which wake, stop and reset keep.
// +demi:tolerant
type CloudDevice struct {
	ID   DeviceID `json:"id"`
	Name string   `json:"name"`
}

// The capacities of a Cloud's two writable filesystems in bytes, or the
// most each may grow to: the capacity, not what is used.
// +demi:tolerant
type CloudVolumes struct {
	// +demi:range max=9007199254740991
	SystemBytes uint64 `json:"systemBytes"`
	// +demi:range max=9007199254740991
	HomeBytes uint64 `json:"homeBytes"`
}

// `GET /cloud`, and the `cloud` of the product state: the device, its
// lifecycle state, its latest reset, why its last boot, save or reset
// failed, the capacities of its filesystems once its first boot made them,
// and the most they may grow to. Reading it never wakes the Cloud.
// +demi:root direction=receive output=web
// +demi:tolerant
type CloudStatus struct {
	// +demi:nullable
	Device *CloudDevice `json:"device"`
	State  CloudState   `json:"state"`
	// +demi:nullable
	Operation *CloudOperation `json:"operation"`
	// +demi:nullable
	Error *string `json:"error"`
	// +demi:nullable
	Volumes *CloudVolumes `json:"volumes"`
	Limits  CloudVolumes  `json:"limits"`
}

// `POST /cloud/reset`: the operation the page names the reset by; a retry
// with the same id is the same reset.
// +demi:root direction=send output=web
type CloudReset struct {
	OperationID OperationID `json:"operationId"`
}

// `{ operation }`: the reset a `POST /cloud/reset` admitted, or the one it
// names.
// +demi:root direction=receive output=web
// +demi:tolerant
type CloudResetAnswer struct {
	Operation CloudOperation `json:"operation"`
}
