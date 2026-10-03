package storagetest

import (
	"archive/tar"
	"bytes"
	"crypto/sha256"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/klauspost/compress/zstd"
	"github.com/wspl/demi/internal/machinewire"
)

// Entry is a regular archive file, or a symbolic link when Link is nonempty.
type Entry struct {
	// Kind is a tar entry type; zero selects a regular file or, with Link, a symlink.
	Kind byte
	// Path is the archive entry path.
	Path string
	// Data is the regular file content.
	Data []byte
	// Link is the symbolic link target.
	Link string
}

// Entries returns the small root used by image-import and manager scenarios.
func Entries() []Entry {
	return []Entry{
		{Path: "usr/bin/demi-runner", Data: []byte("runner")},
		{Path: "usr/bin/tini", Data: []byte("tini")},
		{Path: "usr/sbin/init", Link: "../bin/tini"},
		{Path: "etc/skel/.profile", Data: []byte("export EDITOR=vi\n")},
	}
}

// CloudImage is a fixture release directory, owned by its test.
type CloudImage struct {
	// Directory holds the fixture release files.
	Directory string
	// Manifest is the fixture release manifest.
	Manifest machinewire.CloudImageManifest
}

// NewCloudImage writes a tar/zstd release and matching generated manifest.
func NewCloudImage(
	t *testing.T,
	entries []Entry,
	architecture machinewire.Architecture,
	executables ...Entry,
) *CloudImage {
	t.Helper()
	directory := t.TempDir()
	archive := WriteArchive(t, directory, entries)
	compressed, err := os.ReadFile(archive)
	if err != nil {
		t.Fatal(err)
	}
	// Keep storage's checked-in fixture as the single manifest source.
	_, source, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("Cloud fixture source path unavailable")
	}
	manifestFixture, err := os.ReadFile(filepath.Join(filepath.Dir(source), "../testdata/manifest.json"))
	if err != nil {
		t.Fatal(err)
	}
	manifest, err := machinewire.DecodeCloudImageManifest(manifestFixture)
	if err != nil {
		t.Fatal(err)
	}
	runner := manifest.Runner.Targets[manifest.Architecture.Target()]
	delete(manifest.Runner.Targets, manifest.Architecture.Target())
	manifest.Architecture = architecture
	manifest.Runner.Targets[architecture.Target()] = runner
	if len(executables) != 0 {
		artifact := manifest.Executables["/usr/bin/demi-runner"]
		clear(manifest.Executables)
		for _, executable := range executables {
			artifact.SHA256 = fmt.Sprintf("%x", sha256.Sum256(executable.Data))
			artifact.Size = uint64(len(executable.Data))
			manifest.Executables[executable.Path] = artifact
		}
	}
	manifest.Rootfs.Size = uint64(len(compressed))
	manifest.Rootfs.SHA256 = fmt.Sprintf("%x", sha256.Sum256(compressed))
	image := &CloudImage{Directory: directory, Manifest: manifest}
	image.Write(t)
	return image
}

// Write persists the manifest after a test changes it, including invalid cases.
func (i *CloudImage) Write(t *testing.T) {
	t.Helper()
	data, err := i.Manifest.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(filepath.Join(i.Directory, "manifest.json"), data, 0o600); err != nil {
		t.Fatal(err)
	}
}

// EditBytes allows intentionally malformed JSON without declaring another schema.
func (i *CloudImage) EditBytes(t *testing.T, change func([]byte) []byte) {
	t.Helper()
	path := filepath.Join(i.Directory, "manifest.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(path, change(data), 0o600); err != nil {
		t.Fatal(err)
	}
}

// WriteArchive creates a Cloud tar/zstd fixture, including deliberately unsafe entries.
func WriteArchive(t *testing.T, directory string, entries []Entry) string {
	t.Helper()
	var compressed bytes.Buffer
	encoder, err := zstd.NewWriter(&compressed, zstd.WithEncoderConcurrency(1))
	if err != nil {
		t.Fatal(err)
	}
	// Deferred closes release compressor resources even when fixture creation fails.
	defer func() { _ = encoder.Close() }()
	writer := tar.NewWriter(encoder)
	defer func() { _ = writer.Close() }()
	for _, entry := range entries {
		kind := entry.Kind
		if kind == 0 {
			kind = tar.TypeReg
		}
		if entry.Kind == 0 && entry.Link != "" {
			kind = tar.TypeSymlink
		}
		if err = writer.WriteHeader(
			&tar.Header{
				Name:     entry.Path,
				Typeflag: kind,
				Linkname: entry.Link,
				Mode:     0o755,
				Size:     int64(len(entry.Data)),
			},
		); err != nil {
			t.Fatal(err)
		}
		if _, err = writer.Write(entry.Data); err != nil {
			t.Fatal(err)
		}
	}
	if err = writer.Close(); err != nil {
		t.Fatal(err)
	}
	if err = encoder.Close(); err != nil {
		t.Fatal(err)
	}

	path := filepath.Join(directory, "rootfs.tar.zst")
	if err := os.WriteFile(path, compressed.Bytes(), 0o600); err != nil {
		t.Fatal(err)
	}
	return path
}
