//go:build linux

package storage_test

import (
	"crypto/sha256"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/machines/storage"
	"github.com/wspl/demi/internal/machines/storage/storagetest"
	"github.com/wspl/demi/internal/machinewire"
)

// Cost: one bsdtar extraction and filesystem sync; normally <1 s in the VM.
func TestVerifiedBaseImportedOnceUnderManifestDigest(t *testing.T) {
	tools := storageTools(t)
	architecture, ok := machinewire.HostArchitecture()
	if !ok {
		t.Fatal("unsupported host architecture")
	}
	image := storagetest.NewCloudImage(t, storagetest.Entries(), architecture)
	image.Manifest.Executables["/usr/sbin/init"] = image.Manifest.Executables[machinewire.InitPath]
	image.Write(t)
	data, err := os.ReadFile(filepath.Join(image.Directory, "manifest.json"))
	requireStorage(t, err)
	bases := t.TempDir()
	version, err := storage.ImportBase(t.Context(), tools, image.Directory, bases)
	requireStorage(t, err)
	if string(version) != fmt.Sprintf("%x", sha256.Sum256(data)) {
		t.Fatal(version)
	}
	base := filepath.Join(bases, string(version))
	for path, want := range map[string]string{"manifest.json": string(data), "rootfs/usr/bin/tini": "tini"} {
		got, err := os.ReadFile(filepath.Join(base, path))
		requireStorage(t, err)
		if string(got) != want {
			t.Fatalf("%s: %q", path, got)
		}
	}
	if _, err := os.Stat(filepath.Join(base, "rootfs.tar.zst")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("archive retained: %v", err)
	}
	requireStorage(t, os.Mkdir(filepath.Join(bases, ".base-stale"), 0o700))
	again, err := storage.ImportBase(t.Context(), tools, image.Directory, bases)
	requireStorage(t, err)
	if again != version {
		t.Fatalf("version changed: %s", again)
	}
	entries, err := os.ReadDir(bases)
	requireStorage(t, err)
	if len(entries) != 1 || entries[0].Name() != string(version) {
		t.Fatal(entries)
	}
	requireStorage(t, os.WriteFile(filepath.Join(base, "manifest.json"), []byte("{}"), 0o600))
	_, err = storage.ImportBase(t.Context(), tools, image.Directory, bases)
	if !errors.Is(err, storage.ErrPinnedManifest) {
		t.Fatalf("pinned bytes: %v", err)
	}
}

// Cost: six small import scenarios, up to three bsdtar runs; normally <1 s.
func TestInvalidReleaseRefusedWithoutStage(t *testing.T) {
	tools := storageTools(t)
	for _, name := range []string{
		"archive",
		"architecture",
		"missing-init",
		"absent-executable",
		"executable-digest",
		"escape",
	} {
		t.Run(name, func(t *testing.T) {
			architecture, ok := machinewire.HostArchitecture()
			if !ok {
				t.Fatal("unsupported host architecture")
			}
			members := append(storagetest.Entries(), storagetest.Entry{Path: "usr/bin/escape", Link: "/etc/hostname"})
			image := storagetest.NewCloudImage(t, members, architecture)
			manifest := &image.Manifest
			switch name {
			case "archive":
				manifest.Rootfs.SHA256 = fmt.Sprintf("%064d", 0)
			case "architecture":
				runner := manifest.Runner.Targets[manifest.Architecture.Target()]
				delete(manifest.Runner.Targets, manifest.Architecture.Target())
				if manifest.Architecture == machinewire.ArchitectureARM64 {
					manifest.Architecture = machinewire.ArchitectureAMD64
				} else {
					manifest.Architecture = machinewire.ArchitectureARM64
				}
				manifest.Runner.Targets[manifest.Architecture.Target()] = runner
			case "missing-init":
				delete(manifest.Executables, machinewire.InitPath)
			case "absent-executable":
				manifest.Executables["/usr/bin/demi-helper"] = manifest.Executables[machinewire.InitPath]
			case "executable-digest":
				artifact := manifest.Executables[machinewire.InitPath]
				artifact.SHA256 = fmt.Sprintf("%x", sha256.Sum256([]byte("else")))
				manifest.Executables[machinewire.InitPath] = artifact
			case "escape":
				manifest.Executables["/usr/bin/escape"] = manifest.Executables[machinewire.InitPath]
			}
			image.Write(t)
			bases := t.TempDir()
			_, err := storage.ImportBase(t.Context(), tools, image.Directory, bases)
			matches := false
			switch name {
			case "archive":
				matches = err != nil && strings.Contains(err.Error(), "Cloud root archive integrity mismatch: ")
			case "architecture":
				matches = errors.Is(err, storage.ErrArchitecture)
			case "missing-init":
				matches = err != nil &&
					strings.Contains(err.Error(), "Cloud image manifest lacks "+machinewire.InitPath)
			case "absent-executable":
				matches = errors.Is(err, os.ErrNotExist)
			case "executable-digest":
				matches = err != nil &&
					strings.Contains(err.Error(), "Cloud executable integrity mismatch: "+machinewire.InitPath)
			case "escape":
				matches = err != nil && strings.Contains(err.Error(), "Invalid image executable path: /usr/bin/escape")
			}
			if !matches {
				t.Fatalf("%s: unexpected failure %v", name, err)
			}
			entries, readErr := os.ReadDir(bases)
			requireStorage(t, readErr)
			if len(entries) != 0 {
				t.Fatalf("stage left: %v", entries)
			}
		})
	}
}
