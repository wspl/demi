//go:build linux

package storage_test

import (
	"archive/tar"
	"crypto/sha256"
	_ "embed"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/machines/storage"
	"github.com/wspl/demi/internal/machinewire"
)

//go:embed testdata/manifest.json
var baseManifestFixture []byte

// baseRelease creates a verified Cloud fixture from the Rust testing::entries scenario.
// The checked-in contract fixture avoids a second hand-written manifest shape.
func baseRelease(t *testing.T, extra ...archiveMember) (string, machinewire.CloudImageManifest) {
	t.Helper()
	directory := t.TempDir()
	entries := []archiveMember{
		{kind: tar.TypeReg, name: "usr/bin/demi-runner", data: []byte("runner")},
		{kind: tar.TypeReg, name: "usr/bin/tini", data: []byte("tini")},
		{kind: tar.TypeSymlink, name: "usr/sbin/init", link: "../bin/tini"},
		{kind: tar.TypeReg, name: "etc/skel/.profile", data: []byte("export EDITOR=vi\n")},
	}
	archive := cloudArchive(t, directory, append(entries, extra...))
	manifest, err := machinewire.DecodeCloudImageManifest(baseManifestFixture)
	requireStorage(t, err)
	runner := manifest.Runner.Targets[manifest.Architecture.Target()]
	delete(manifest.Runner.Targets, manifest.Architecture.Target())
	architecture, ok := machinewire.HostArchitecture()
	if !ok {
		t.Fatal("unsupported host architecture")
	}
	manifest.Architecture = architecture
	manifest.Runner.Targets[architecture.Target()] = runner
	compressed, err := os.ReadFile(archive)
	requireStorage(t, err)
	manifest.Rootfs.Size = uint64(len(compressed))
	manifest.Rootfs.SHA256 = fmt.Sprintf("%x", sha256.Sum256(compressed))
	return directory, manifest
}

// writeBaseManifest records a fixture through the contract's encoder.
func writeBaseManifest(t *testing.T, directory string, manifest machinewire.CloudImageManifest) []byte {
	t.Helper()
	data, err := contract.EncodeJSON(manifest)
	requireStorage(t, err)
	requireStorage(t, os.WriteFile(filepath.Join(directory, "manifest.json"), data, 0600))
	return data
}

// Cost: one bsdtar extraction and filesystem sync; normally <1 s in the VM.
func TestVerifiedBaseImportedOnceUnderManifestDigest(t *testing.T) {
	tools := storageTools(t)
	image, manifest := baseRelease(t)
	manifest.Executables["/usr/sbin/init"] = manifest.Executables[machinewire.InitPath]
	data := writeBaseManifest(t, image, manifest)
	bases := t.TempDir()
	version, err := storage.ImportBase(t.Context(), tools, image, bases)
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
	requireStorage(t, os.Mkdir(filepath.Join(bases, ".base-stale"), 0700))
	again, err := storage.ImportBase(t.Context(), tools, image, bases)
	requireStorage(t, err)
	if again != version {
		t.Fatalf("version changed: %s", again)
	}
	entries, err := os.ReadDir(bases)
	requireStorage(t, err)
	if len(entries) != 1 || entries[0].Name() != string(version) {
		t.Fatal(entries)
	}
	requireStorage(t, os.WriteFile(filepath.Join(base, "manifest.json"), []byte("{}"), 0600))
	_, err = storage.ImportBase(t.Context(), tools, image, bases)
	if !errors.Is(err, storage.ErrPinnedManifest) {
		t.Fatalf("pinned bytes: %v", err)
	}
}

// Cost: six small import scenarios, up to three bsdtar runs; normally <1 s.
func TestInvalidReleaseRefusedWithoutStage(t *testing.T) {
	tools := storageTools(t)
	for _, name := range []string{"archive", "architecture", "missing-init", "absent-executable", "executable-digest", "escape"} {
		t.Run(name, func(t *testing.T) {
			image, manifest := baseRelease(t, archiveMember{kind: tar.TypeSymlink, name: "usr/bin/escape", link: "/etc/hostname"})
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
			writeBaseManifest(t, image, manifest)
			bases := t.TempDir()
			_, err := storage.ImportBase(t.Context(), tools, image, bases)
			var integrity *storage.ArchiveIntegrityError
			var missing *storage.MissingExecutableError
			var executable *storage.ExecutableIntegrityError
			var escape *storage.ExecutablePathError
			matches := false
			switch name {
			case "archive":
				matches = errors.As(err, &integrity)
			case "architecture":
				matches = errors.Is(err, storage.ErrArchitecture)
			case "missing-init":
				matches = errors.As(err, &missing) && missing.Path == machinewire.InitPath
			case "absent-executable":
				matches = errors.Is(err, os.ErrNotExist)
			case "executable-digest":
				matches = errors.As(err, &executable) && executable.Path == machinewire.InitPath
			case "escape":
				matches = errors.As(err, &escape) && escape.Path == "/usr/bin/escape"
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
