//go:build linux

package storage

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machinewire"
)

// NewGeneration returns a new generation ID, reporting a failure to obtain randomness.
func NewGeneration() (machinewire.GenerationID, error) { panic("not written: m-storage") }

// WorkingPair locates a device's private writable images and records.
// The manager serializes its operations and fences writers before Save.
type WorkingPair struct{}

// NewWorkingPair locates device's pair under the working directory.
func NewWorkingPair(working string, device machinewire.DeviceID) *WorkingPair {
	panic("not written: m-storage")
}

// Directory returns the working pair's directory.
func (p *WorkingPair) Directory() string { panic("not written: m-storage") }

// Images returns the working image paths.
func (p *WorkingPair) Images() ImagePair[string] { panic("not written: m-storage") }

// SandboxRecord returns the runtime record path, sandbox.json.
func (p *WorkingPair) SandboxRecord() string { panic("not written: m-storage") }

// Manifest reads the working pair's record, or nil when there is none.
func (p *WorkingPair) Manifest(ctx context.Context) (*machinewire.MachineImageState, error) {
	panic("not written: m-storage")
}

// WriteManifest replaces the working record durably and syncs its directory.
func (p *WorkingPair) WriteManifest(ctx context.Context, state machinewire.MachineImageState) error {
	panic("not written: m-storage")
}

// Save publishes the working pair as a new generation and removes it;
// nothing happens without a manifest. Nothing may write the images: each is
// checked, interrupted growth completed, and synced before publication.
func (p *WorkingPair) Save(ctx context.Context, tools *system.Tools, store *Store, device machinewire.DeviceID) error {
	panic("not written: m-storage")
}
