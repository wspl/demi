//go:build linux

package storage_test

import (
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/machines/storage"
	"github.com/wspl/demi/internal/machinewire"
)

// generationState supplies a small valid paired generation for publication tests.
func generationState(t *testing.T, name string) machinewire.MachineImageState {
	t.Helper()
	id, err := machinewire.ParseGenerationID(name)
	requireStorage(t, err)
	return machinewire.MachineImageState{Generation: id, BaseVersion: "base", SystemBytes: 1024, HomeBytes: 1024}
}

// generationNames observes the generations visible to recovery, including stale stages.
func generationNames(t *testing.T, root string) []string {
	t.Helper()
	entries, err := os.ReadDir(filepath.Join(root, "device/generations"))
	requireStorage(t, err)
	names := make([]string, 0, len(entries))
	for _, entry := range entries {
		names = append(names, entry.Name())
	}
	return names
}

// Cost: local filesystem IO only; no processes or waits.
func TestPublicationKeepsCommittedPairAndTwoGenerations(t *testing.T) {
	ctx := t.Context()
	root := filepath.Join(t.TempDir(), "images")
	requireStorage(t, os.Mkdir(root, 0o700))
	store := storage.NewStore(root)
	source := storage.ImagesInDirectory(t.TempDir())
	requireStorage(t, os.WriteFile(source.System, []byte("system"), 0o600))
	requireStorage(t, os.WriteFile(source.Home, []byte("home"), 0o600))
	_, found, err := store.Read(ctx, "device")
	requireStorage(t, err)
	if found {
		t.Fatal("new device has a generation")
	}
	requireStorage(t, store.Publish(ctx, "device", generationState(t, "first"), source))
	requireStorage(t, os.Remove(source.Home))
	if err := store.Publish(ctx, "device", generationState(t, "partial"), source); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("missing home: %v", err)
	}
	state, _, err := store.Read(ctx, "device")
	requireStorage(t, err)
	if !reflect.DeepEqual(state, generationState(t, "first")) {
		t.Fatalf("changed committed state: %+v", state)
	}
	if got := generationNames(t, root); !reflect.DeepEqual(got, []string{"first"}) {
		t.Fatal(got)
	}
	committed := store.Images("device", "first")
	for _, volume := range []machinewire.Volume{machinewire.VolumeSystem, machinewire.VolumeHome} {
		data, err := os.ReadFile(committed.ForVolume(volume))
		requireStorage(t, err)
		if string(data) != string(volume) {
			t.Fatalf("lost %s content", volume)
		}
		// Replace sources rather than modifying an image still linked in a generation.
		requireStorage(t, os.RemoveAll(source.ForVolume(volume)))
		requireStorage(t, os.WriteFile(source.ForVolume(volume), data, 0o600))
	}
	requireStorage(t, store.Publish(ctx, "device", generationState(t, "second"), source))
	requireStorage(t, os.Remove(source.System))
	requireStorage(t, os.Remove(source.Home))
	requireStorage(t, os.Mkdir(filepath.Join(root, "device/generations/.publish-stale"), 0o700))
	requireStorage(t, store.Publish(ctx, "device", generationState(t, "third"), store.Images("device", "second")))
	if got := generationNames(t, root); !reflect.DeepEqual(got, []string{"second", "third"}) {
		t.Fatal(got)
	}
	state, _, err = store.Read(ctx, "device")
	requireStorage(t, err)
	if !reflect.DeepEqual(state, generationState(t, "third")) {
		t.Fatal(state)
	}
}

// Cost: local filesystem IO only.
func TestPublicationLinksSources(t *testing.T) {
	ctx := t.Context()
	store := storage.NewStore(t.TempDir())
	sources := storage.ImagesInDirectory(t.TempDir())
	requireStorage(t, os.WriteFile(sources.System, []byte("system"), 0o600))
	requireStorage(t, os.WriteFile(sources.Home, []byte("home"), 0o600))
	requireStorage(t, store.Publish(ctx, "device", generationState(t, "first"), sources))
	committed := store.Images("device", "first")
	for _, volume := range []machinewire.Volume{machinewire.VolumeSystem, machinewire.VolumeHome} {
		source, err := os.Stat(sources.ForVolume(volume))
		requireStorage(t, err)
		image, err := os.Stat(committed.ForVolume(volume))
		requireStorage(t, err)
		if !os.SameFile(source, image) {
			t.Fatalf("%s was copied, not linked", volume)
		}
	}
}

// Cost: local filesystem IO only; corrupt input must remain untouched.
func TestCorruptRecordIsError(t *testing.T) {
	root := t.TempDir()
	store := storage.NewStore(root)
	requireStorage(t, os.Mkdir(filepath.Join(root, "device"), 0o700))
	path := filepath.Join(root, "device", "current.json")
	data := []byte(`{"generation":"g"}`)
	requireStorage(t, os.WriteFile(path, data, 0o600))
	_, _, err := store.Read(t.Context(), "device")
	if err == nil || !strings.HasPrefix(err.Error(), path+" is not a valid generation record: ") {
		t.Fatalf("corrupt record: %v", err)
	}
	kept, err := os.ReadFile(path)
	requireStorage(t, err)
	if string(kept) != string(data) {
		t.Fatal("corrupt record repaired")
	}
}
