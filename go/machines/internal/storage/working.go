package storage

import (
	"context"
	"path/filepath"

	"github.com/wspl/demi/go/machines/internal/fault"
	"github.com/wspl/demi/go/machines/internal/tools"
	"github.com/wspl/demi/go/machinesproto"
)

// A WorkingPair is a device's working pair (docs/cloud/managed-hosts.md §
// Images): the private writable copies of its committed images, with a manifest
// that names the generation they came from and their current capacities.
//
//	<data>/working/<device>/manifest.json   the working pair's record
//	<data>/working/<device>/system.ext4, home.ext4
//	<data>/working/<device>/sandbox.json    while a sandbox runs on it
type WorkingPair struct {
	directory string
}

// NewWorkingPair returns device's working pair under the working directory.
func NewWorkingPair(working string, device machinesproto.DeviceID) *WorkingPair {
	return &WorkingPair{directory: filepath.Join(working, string(device))}
}

// Directory returns the pair's directory.
func (w *WorkingPair) Directory() string { return w.directory }

// Images returns the pair's images.
func (w *WorkingPair) Images() ImagePair[string] { return ImagesIn(w.directory) }

// SandboxRecord returns the path of the runtime record.
func (w *WorkingPair) SandboxRecord() string { return filepath.Join(w.directory, "sandbox.json") }

// Manifest returns the working pair's record, nil when the device has no working
// pair.
func (w *WorkingPair) Manifest() (*machinesproto.ImageState, error) {
	return ReadState(filepath.Join(w.directory, "manifest.json"))
}

// WriteManifest replaces the record durably.
func (w *WorkingPair) WriteManifest(state machinesproto.ImageState) error {
	if err := WriteState(filepath.Join(w.directory, "manifest.json"), state); err != nil {
		return err
	}
	return Sync(w.directory)
}

// Save publishes the working pair as device's new generation and removes it;
// nothing happens without one. Nothing may write the images: each is checked, an
// interrupted growth completed, and each synced, then linked into the
// generation.
func (w *WorkingPair) Save(ctx context.Context, t *tools.Tools, store *Store, device machinesproto.DeviceID) error {
	state, err := w.Manifest()
	if err != nil || state == nil {
		return err
	}
	images := w.Images()
	systemBytes, err := Recover(ctx, t, images.System)
	if err != nil {
		return err
	}
	homeBytes, err := Recover(ctx, t, images.Home)
	if err != nil {
		return err
	}
	saved := *state
	saved.Generation = NewGeneration()
	saved.SystemBytes = systemBytes
	saved.HomeBytes = homeBytes
	if err := store.Publish(device, saved, images); err != nil {
		return err
	}
	fault.Point("working-published")
	if err := RemoveTree(w.directory); err != nil {
		return err
	}
	return Sync(filepath.Dir(w.directory))
}
