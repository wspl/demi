package machinewire

// A persistent Cloud device, as the backend names it. It names the
// device's image and working directories.
// +demi:root
// +demi:id
// +demi:pattern ^[A-Za-z0-9_-]+$
type DeviceID string

// One committed pair of a device's system and home images.
// +demi:root
// +demi:id
// +demi:pattern ^[A-Za-z0-9_-]+$
type GenerationID string

// An imported base: the SHA-256 of its image manifest's bytes.
// +demi:root
// +demi:id
// +demi:pattern ^[A-Za-z0-9_-]+$
type BaseVersion string

// A device's committed generation: its base, the reset that made it, and
// the capacities of its two filesystems. The same record answers
// `image_state`, names the generation in `current.json`, and describes the
// generation and the working pair on disk.
// +demi:root
type MachineImageState struct {
	Generation  GenerationID `json:"generation"`
	BaseVersion BaseVersion  `json:"baseVersion"`
	// The reset operation that published this generation's system image,
	// or `null` before the first reset. The key is always written.
	// +demi:nullable
	ResetID *string `json:"resetId"`
	// The system filesystem's capacity in bytes, not its use.
	// +demi:range min=1
	SystemBytes uint64 `json:"systemBytes"`
	// The home filesystem's capacity in bytes, not its use.
	// +demi:range min=1
	HomeBytes uint64 `json:"homeBytes"`
}

// Whether the manager runs a sandbox for a device.
// +demi:root
// +demi:enum running stopped
type RuntimeState string

const (
	RuntimeStateRunning RuntimeState = "running"
	RuntimeStateStopped RuntimeState = "stopped"
)

// One of a device's two writable filesystems.
// +demi:root
// +demi:enum system home
type Volume string

const (
	VolumeSystem Volume = "system"
	VolumeHome   Volume = "home"
)
