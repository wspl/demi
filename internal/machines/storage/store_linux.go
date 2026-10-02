//go:build linux

package storage

import (
	"context"
	"errors"
	"log/slog"
	"os"
	"path/filepath"

	"github.com/wspl/demi/internal/machines/system"
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
func (p ImagePair[T]) ForVolume(volume machinewire.Volume) T {
	if volume == machinewire.VolumeSystem {
		return p.System
	}
	return p.Home
}

// ImageFile returns system.ext4 or home.ext4 for a validated volume.
func ImageFile(volume machinewire.Volume) string { return string(volume) + ".ext4" }

// ImagesInDirectory names the system.ext4 and home.ext4 images in directory.
func ImagesInDirectory(directory string) ImagePair[string] {
	return ImagePair[string]{System: filepath.Join(directory, ImageFile(machinewire.VolumeSystem)), Home: filepath.Join(directory, ImageFile(machinewire.VolumeHome))}
}

// Store holds the committed generations under <data>/images.
// The manager serializes operations on each device.
type Store struct{ root string }

// NewStore locates the committed image store at root.
func NewStore(root string) *Store { return &Store{root: root} }

// Bases returns the imported bases directory, <data>/images/bases.
func (s *Store) Bases() string { return filepath.Join(s.root, "bases") }

// ReadState reads a generation record; nil means the file does not exist.
// A record that does not decode is an error; nothing repairs it.
func ReadState(ctx context.Context, path string) (*machinewire.MachineImageState, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	data, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	state, err := machinewire.DecodeMachineImageState(data)
	if err != nil {
		return nil, &CorruptRecordError{Path: path, Source: err}
	}
	return &state, nil
}

// Read returns a device's committed generation, or nil before its first.
func (s *Store) Read(ctx context.Context, device machinewire.DeviceID) (*machinewire.MachineImageState, error) {
	return ReadState(ctx, filepath.Join(s.root, string(device), "current.json"))
}

// Images returns the images of one of a device's generations.
func (s *Store) Images(device machinewire.DeviceID, generation machinewire.GenerationID) ImagePair[string] {
	return ImagesInDirectory(filepath.Join(s.root, string(device), "generations", string(generation)))
}

// Publish commits state's generation with sources as its images, then keeps
// only it and the generation it replaces. Sources must be synced, on this
// filesystem, and never written again: publication hard-links them.
// Failure before replacing current.json preserves the committed generation.
// Once publication begins its commit and cleanup complete despite cancellation.
func (s *Store) Publish(ctx context.Context, device machinewire.DeviceID, state machinewire.MachineImageState, sources ImagePair[string]) error {
	previous, err := s.Read(ctx, device)
	if err != nil {
		return err
	}
	ctx = context.WithoutCancel(ctx)
	directory := filepath.Join(s.root, string(device))
	generations := filepath.Join(directory, "generations")
	if err := os.MkdirAll(generations, 0777); err != nil {
		return err
	}
	if err := Sync(ctx, s.root); err != nil {
		return err
	}
	if err := Sync(ctx, directory); err != nil {
		return err
	}
	id, err := NewGeneration()
	if err != nil {
		return err
	}
	stage := filepath.Join(generations, ".publish-"+string(id))
	if err := os.Mkdir(stage, 0777); err != nil {
		return err
	}
	// The unpublished stage is removed even when staging or rename fails.
	defer func() {
		if err := RemoveTree(ctx, stage); err != nil {
			slog.Warn("machines: " + err.Error())
		}
	}()
	if err := stageGeneration(ctx, stage, state, sources); err != nil {
		return err
	}
	system.FaultPoint("generation-staged")
	if err := os.Rename(stage, filepath.Join(generations, string(state.Generation))); err != nil {
		return err
	}
	if err := Sync(ctx, generations); err != nil {
		return err
	}
	system.FaultPoint("generation-renamed")
	if err := WriteJSON(ctx, filepath.Join(directory, "current.json"), state); err != nil {
		return err
	}
	if err := Sync(ctx, directory); err != nil {
		return err
	}
	entries, err := os.ReadDir(generations)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		keep := entry.Name() == string(state.Generation) || previous != nil && entry.Name() == string(previous.Generation)
		if !keep && entry.IsDir() {
			if err := RemoveTree(ctx, filepath.Join(generations, entry.Name())); err != nil {
				return err
			}
		}
	}
	return Sync(ctx, generations)
}

// stageGeneration links immutable images and records their paired generation.
func stageGeneration(ctx context.Context, stage string, state machinewire.MachineImageState, sources ImagePair[string]) error {
	for _, volume := range []machinewire.Volume{machinewire.VolumeSystem, machinewire.VolumeHome} {
		if err := os.Link(sources.ForVolume(volume), filepath.Join(stage, ImageFile(volume))); err != nil {
			return err
		}
	}
	if err := WriteJSON(ctx, filepath.Join(stage, "manifest.json"), state); err != nil {
		return err
	}
	return Sync(ctx, stage)
}
