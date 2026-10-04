package machineproto

import "fmt"

// IsImageName reports whether value may name a device, a generation or a base: one or more
// ASCII letters, digits, `_` or `-`. Such a name is always one path
// component, so it names a directory under the manager's state directory.
func IsImageName(value string) bool {
	_, err := ParseDeviceID(value)
	return err == nil
}

// Bytes returns the recorded capacity of volume.
func (s MachineImageState) Bytes(volume Volume) (uint64, error) {
	switch volume {
	case VolumeSystem:
		return s.SystemBytes, nil
	case VolumeHome:
		return s.HomeBytes, nil
	default:
		return 0, fmt.Errorf("unknown volume %q", volume)
	}
}

// WithBytes returns the record with volume's capacity replaced.
func (s MachineImageState) WithBytes(volume Volume, size uint64) (MachineImageState, error) {
	if size == 0 {
		return MachineImageState{}, fmt.Errorf("volume capacity must be nonzero")
	}
	switch volume {
	case VolumeSystem:
		s.SystemBytes = size
	case VolumeHome:
		s.HomeBytes = size
	default:
		return MachineImageState{}, fmt.Errorf("unknown volume %q", volume)
	}
	return s, nil
}

// Volumes returns both volumes, system first.
func Volumes() [2]Volume {
	return [2]Volume{VolumeSystem, VolumeHome}
}
