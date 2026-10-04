//go:build linux

package storage_test

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/machinemanager/storage"
	"github.com/wspl/demi/internal/machinemanagerproto"
)

// Cost: two sparse ext4 images, checks and one resize, normally <1 s.
// A failed home save retains both working images; a retry publishes their pair.
func TestWorkingSaveRetainsFailureAndPublishesRecoveredPair(t *testing.T) {
	ctx := t.Context()
	tools := storageTools(t)
	store := storage.NewStore(t.TempDir())
	pair := storage.NewWorkingPair(t.TempDir(), "device")
	requireStorage(t, pair.Save(ctx, tools, store, "device"))
	requireStorage(t, storage.CreatePrivate(ctx, pair.Directory()))
	images := pair.Images()
	requireStorage(t, storage.MakeSystem(ctx, tools, images.System, 32<<20))
	requireStorage(t, os.WriteFile(images.Home, []byte("broken ext4"), 0o600))
	state := generationState(t, "working")
	reset := "reset<&>\u2028\u2029"
	state.ResetID = &reset
	state.SystemBytes = 32 << 20
	state.HomeBytes = 32 << 20
	requireStorage(t, pair.WriteManifest(ctx, state))
	raw, err := os.ReadFile(filepath.Join(pair.Directory(), "manifest.json"))
	requireStorage(t, err)
	if !strings.Contains(string(raw), reset) {
		t.Fatalf("record changed JSON string bytes: %s", raw)
	}
	if err := pair.Save(ctx, tools, store, "device"); err == nil {
		t.Fatal("published corrupt home")
	}
	kept, found, err := pair.Manifest(ctx)
	requireStorage(t, err)
	if !found || kept.Generation != state.Generation {
		t.Fatal("lost working record")
	}
	committed, found, err := store.Read(ctx, "device")
	requireStorage(t, err)
	if found {
		t.Fatal("published a partial pair")
	}
	requireStorage(t, os.Remove(images.Home))
	home := t.TempDir()
	requireStorage(t, os.WriteFile(filepath.Join(home, "kept.txt"), []byte("home survives\n"), 0o644))
	requireStorage(t, storage.MakeHome(ctx, tools, home, images.Home, 32<<20))
	requireStorage(t, os.Truncate(images.System, 64<<20))
	requireStorage(t, pair.Save(ctx, tools, store, "device"))
	committed, found, err = store.Read(ctx, "device")
	requireStorage(t, err)
	if !found || committed.Generation == state.Generation || committed.SystemBytes != 64<<20 ||
		committed.HomeBytes != 32<<20 ||
		committed.BaseVersion != state.BaseVersion ||
		committed.ResetID == nil ||
		*committed.ResetID != reset {
		t.Fatalf("saved state: %+v", committed)
	}
	if _, err := machinemanagerproto.ParseGenerationID(string(committed.Generation)); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(pair.Directory()); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("working data left after save: %v", err)
	}
	saved := store.Images("device", committed.Generation)
	content, err := diskCommand(ctx, "debugfs", "-R", "cat /kept.txt", saved.Home)
	requireStorage(t, err)
	if string(content) != "home survives\n" {
		t.Fatalf("saved home: %q", content)
	}
	requireStorage(t, pair.Save(ctx, tools, store, "device"))
}

// Cost: local record IO only. A cancelled operation must not replace a record.
func TestCancelledRecordWritePreservesPreviousState(t *testing.T) {
	pair := storage.NewWorkingPair(t.TempDir(), "device")
	requireStorage(t, storage.CreatePrivate(t.Context(), pair.Directory()))
	requireStorage(t, pair.WriteManifest(t.Context(), generationState(t, "first")))
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if err := pair.WriteManifest(ctx, generationState(t, "second")); !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
	state, found, err := pair.Manifest(t.Context())
	requireStorage(t, err)
	if !found || state.Generation != "first" {
		t.Fatal(state)
	}
}
