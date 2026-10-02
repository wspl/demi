//go:build linux

package storage

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/machinewire"
)

// ImagePair holds a value for each of a device's two volumes.
type ImagePair[T any] struct {
	// System is the writable system layer's value.
	System T
	// Home is the home filesystem's value.
	Home T
}

// ForVolume returns the value for volume, which must be a validated Volume.
func (p ImagePair[T]) ForVolume(volume machinewire.Volume) T { panic("not written: m-storage") }

// ImageFile returns system.ext4 or home.ext4 for a validated volume.
func ImageFile(volume machinewire.Volume) string { panic("not written: m-storage") }

// ImagesInDirectory names the system.ext4 and home.ext4 images in directory.
func ImagesInDirectory(directory string) ImagePair[string] { panic("not written: m-storage") }

// Store holds the committed generations under <data>/images.
// The manager serializes operations on each device.
type Store struct{}

// NewStore locates the committed image store at root.
func NewStore(root string) *Store { panic("not written: m-storage") }

// Bases returns the imported bases directory, <data>/images/bases.
func (s *Store) Bases() string { panic("not written: m-storage") }

// ReadState reads a generation record; nil means the file does not exist.
// A record that does not decode is an error; nothing repairs it.
func ReadState(ctx context.Context, path string) (*machinewire.MachineImageState, error) {
	panic("not written: m-storage")
}

// Read returns a device's committed generation, or nil before its first.
func (s *Store) Read(ctx context.Context, device machinewire.DeviceID) (*machinewire.MachineImageState, error) {
	panic("not written: m-storage")
}

// Images returns the images of one of a device's generations.
func (s *Store) Images(device machinewire.DeviceID, generation machinewire.GenerationID) ImagePair[string] {
	panic("not written: m-storage")
}

// Publish commits state's generation with sources as its images, then keeps
// only it and the generation it replaces. Sources must be synced, on this
// filesystem, and never written again: publication hard-links them.
// Failure before replacing current.json preserves the committed generation.
// Once publication begins its commit and cleanup complete despite cancellation.
func (s *Store) Publish(ctx context.Context, device machinewire.DeviceID, state machinewire.MachineImageState, sources ImagePair[string]) error {
	panic("not written: m-storage")
}
