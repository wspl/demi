//go:build linux

package storage

import (
	"context"
	"crypto/rand"
	"fmt"
	"path/filepath"

	"github.com/wspl/demi/internal/machines/system"
	"github.com/wspl/demi/internal/machinewire"
)

// NewGeneration returns a new generation ID, reporting a failure to obtain randomness.
func NewGeneration() (machinewire.GenerationID, error) {
	var id [16]byte
	if _, err := rand.Read(id[:]); err != nil {
		return "", err
	}
	id[6] = id[6]&0x0f | 0x40
	id[8] = id[8]&0x3f | 0x80
	return machinewire.ParseGenerationID(fmt.Sprintf("%x-%x-%x-%x-%x", id[:4], id[4:6], id[6:8], id[8:10], id[10:]))
}

// WorkingPair locates a device's private writable images and records.
// The manager serializes its operations and fences writers before Save.
type WorkingPair struct{ directory string }

// NewWorkingPair locates device's pair under the working directory.
func NewWorkingPair(working string, device machinewire.DeviceID) *WorkingPair {
	return &WorkingPair{directory: filepath.Join(working, string(device))}
}

// Directory returns the working pair's directory.
func (p *WorkingPair) Directory() string { return p.directory }

// Images returns the working image paths.
func (p *WorkingPair) Images() ImagePair[string] { return ImagesInDirectory(p.directory) }

// SandboxRecord returns the runtime record path, sandbox.json.
func (p *WorkingPair) SandboxRecord() string { return filepath.Join(p.directory, "sandbox.json") }

// Manifest reads the working pair's record, or nil when there is none.
func (p *WorkingPair) Manifest(ctx context.Context) (*machinewire.MachineImageState, error) {
	return ReadState(ctx, filepath.Join(p.directory, "manifest.json"))
}

// WriteManifest replaces the working record durably and syncs its directory.
func (p *WorkingPair) WriteManifest(ctx context.Context, state machinewire.MachineImageState) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	ctx = context.WithoutCancel(ctx)
	if err := WriteJSON(ctx, filepath.Join(p.directory, "manifest.json"), state); err != nil {
		return err
	}
	return Sync(ctx, p.directory)
}

// Save publishes the working pair as a new generation and removes it;
// nothing happens without a manifest. Nothing may write the images: each is
// checked, interrupted growth completed, and synced before publication.
func (p *WorkingPair) Save(ctx context.Context, tools *system.Tools, store *Store, device machinewire.DeviceID) error {
	state, err := p.Manifest(ctx)
	if err != nil || state == nil {
		return err
	}
	ctx = context.WithoutCancel(ctx)
	images := p.Images()
	state.SystemBytes, err = Recover(ctx, tools, images.System)
	if err != nil {
		return err
	}
	state.HomeBytes, err = Recover(ctx, tools, images.Home)
	if err != nil {
		return err
	}
	state.Generation, err = NewGeneration()
	if err != nil {
		return err
	}
	if err := store.Publish(ctx, device, *state, images); err != nil {
		return err
	}
	system.FaultPoint("working-published")
	if err := RemoveTree(ctx, p.directory); err != nil {
		return err
	}
	return Sync(ctx, filepath.Dir(p.directory))
}
