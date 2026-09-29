package storage

import (
	"errors"
	"fmt"
	"io/fs"
	"log/slog"
	"os"
	"path/filepath"

	"github.com/google/uuid"

	"github.com/wspl/demi/go/machines/internal/fault"
	"github.com/wspl/demi/go/machinesproto"
)

// A CorruptError means a generation record does not decode. Nothing repairs it.
type CorruptError struct {
	Path string
	Err  error
}

func (e *CorruptError) Error() string {
	return fmt.Sprintf("%s is not a valid generation record: %v", e.Path, e.Err)
}

func (e *CorruptError) Unwrap() error { return e.Err }

// ReadState reads a generation record at path; it returns nil when there is
// none.
func ReadState(path string) (*machinesproto.ImageState, error) {
	data, err := os.ReadFile(path)
	if errors.Is(err, fs.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	state, err := machinesproto.DecodeImageState(data)
	if err != nil {
		return nil, &CorruptError{Path: path, Err: err}
	}
	return &state, nil
}

// WriteState replaces the record at path durably.
func WriteState(path string, state machinesproto.ImageState) error {
	data, err := machinesproto.EncodeImageState(state)
	if err != nil {
		return err
	}
	return WriteRecord(path, data)
}

// NewGeneration returns a new generation's id.
func NewGeneration() machinesproto.GenerationID {
	return machinesproto.GenerationID(uuid.NewString())
}

// A Store is the immutable image store: each device's committed generations and
// the pointer to the current one (docs/cloud/managed-hosts.md § Save a
// generation).
//
//	<data>/images/<device>/current.json               the committed generation
//	<data>/images/<device>/generations/<generation>/  manifest.json, system.ext4, home.ext4
//
// Publication links images into the new generation instead of copying them:
// every source is an image nothing writes any more.
type Store struct {
	root string
}

// NewStore returns the store under root, <data>/images.
func NewStore(root string) *Store {
	return &Store{root: root}
}

// Bases returns the directory of imported bases: <data>/images/bases.
func (s *Store) Bases() string {
	return filepath.Join(s.root, "bases")
}

// Read returns a device's committed generation, nil before its first.
func (s *Store) Read(device machinesproto.DeviceID) (*machinesproto.ImageState, error) {
	return ReadState(filepath.Join(s.root, string(device), "current.json"))
}

// Images returns the images of one of a device's generations.
func (s *Store) Images(device machinesproto.DeviceID, generation machinesproto.GenerationID) ImagePair[string] {
	return ImagesIn(filepath.Join(s.root, string(device), "generations", string(generation)))
}

// Publish commits state's generation with sources as its images, then keeps only
// it and the generation it replaces. Each source is linked, so it must be on
// this filesystem and must never be written again; its producer has synced it. A
// failure before the new current.json leaves the committed generation as it was
// and no stage behind.
func (s *Store) Publish(device machinesproto.DeviceID, state machinesproto.ImageState, sources ImagePair[string]) error {
	previous, err := s.Read(device)
	if err != nil {
		return err
	}
	directory := filepath.Join(s.root, string(device))
	generations := filepath.Join(directory, "generations")
	if err := os.MkdirAll(generations, 0o777); err != nil {
		return err
	}
	if err := Sync(s.root); err != nil {
		return err
	}
	if err := Sync(directory); err != nil {
		return err
	}
	stage := filepath.Join(generations, ".publish-"+uuid.NewString())
	if err := os.Mkdir(stage, 0o777); err != nil {
		return err
	}
	staged := stageGeneration(stage, state, sources)
	if staged == nil {
		fault.Point("generation-staged")
		staged = os.Rename(stage, filepath.Join(generations, string(state.Generation)))
	}
	if staged != nil {
		// A failure to remove the stage is logged beside the first error, which
		// the caller acts on; the next publication removes it.
		if err := RemoveTree(stage); err != nil {
			slog.Warn("machines: " + err.Error())
		}
		return staged
	}
	if err := Sync(generations); err != nil {
		return err
	}
	fault.Point("generation-renamed")
	if err := WriteState(filepath.Join(directory, "current.json"), state); err != nil {
		return err
	}
	if err := Sync(directory); err != nil {
		return err
	}
	// Keep the fallback generation; older ones and stale stages go, which
	// removes only their links to images a newer generation may share.
	entries, err := os.ReadDir(generations)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		name := entry.Name()
		keep := name == string(state.Generation) || previous != nil && name == string(previous.Generation)
		if !keep && entry.IsDir() {
			if err := RemoveTree(filepath.Join(generations, name)); err != nil {
				return err
			}
		}
	}
	return Sync(generations)
}

// stageGeneration links the images into stage, writes the generation's manifest
// and syncs the stage.
func stageGeneration(stage string, state machinesproto.ImageState, sources ImagePair[string]) error {
	for _, volume := range machinesproto.Volumes {
		if err := os.Link(sources.Get(volume), filepath.Join(stage, ImageFile(volume))); err != nil {
			return err
		}
	}
	if err := WriteState(filepath.Join(stage, "manifest.json"), state); err != nil {
		return err
	}
	return Sync(stage)
}
