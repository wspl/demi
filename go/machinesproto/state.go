package machinesproto

// A Volume is one of a device's two writable filesystems.
type Volume string

// The volumes.
const (
	// System is the system layer: the OverlayFS upper and work directories.
	System Volume = "system"
	// Home is /home.
	Home Volume = "home"
)

// Volumes are both volumes, system first.
var Volumes = [2]Volume{System, Home}

// A RuntimeState says whether the manager runs a sandbox for a device.
type RuntimeState string

// The runtime states.
const (
	Running RuntimeState = "running"
	Stopped RuntimeState = "stopped"
)

// An ImageState is a device's committed generation: its base, the reset that
// made it and the capacities of its two filesystems. The same record answers
// image_state, names the generation in current.json and describes the
// generation and the working pair on disk.
//
//demi:wire
type ImageState struct {
	Generation GenerationID `json:"generation" check:"pattern=imageName"`
	// BaseVersion is the base the system layer is made for.
	BaseVersion BaseVersion `json:"baseVersion" check:"pattern=imageName"`
	// ResetID is the reset operation that published this generation's system
	// image, or nil before the first reset. The key is always written.
	ResetID *string `json:"resetId" check:"nullable"`
	// SystemBytes is the system filesystem's capacity in bytes, not its use.
	SystemBytes uint64 `json:"systemBytes" check:"range=1.."`
	// HomeBytes is the home filesystem's capacity in bytes, not its use.
	HomeBytes uint64 `json:"homeBytes" check:"range=1.."`
}

// Bytes returns the recorded capacity of volume.
func (s ImageState) Bytes(volume Volume) uint64 {
	if volume == System {
		return s.SystemBytes
	}
	return s.HomeBytes
}

// WithBytes returns the record with volume's capacity replaced.
func (s ImageState) WithBytes(volume Volume, bytes uint64) ImageState {
	if volume == System {
		s.SystemBytes = bytes
	} else {
		s.HomeBytes = bytes
	}
	return s
}

// DecodeImageState decodes a generation record: every member is required,
// resetId may be null, an unknown member is refused, names follow the
// image-name rule and the capacities are at least 1. A record that does not
// decode is an error; nothing repairs it.
func DecodeImageState(data []byte) (ImageState, error) {
	return decode[ImageState](data)
}

// EncodeImageState returns the record's JSON, compact, with resetId always
// written.
func EncodeImageState(state ImageState) ([]byte, error) {
	return encode(state)
}
