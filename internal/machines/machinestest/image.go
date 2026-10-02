package machinestest

import (
	"archive/tar"
	"bytes"
	"crypto/sha256"
	_ "embed"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/klauspost/compress/zstd"
	"github.com/wspl/demi/internal/machinewire"
)

//go:embed testdata/manifest.json
var manifestFixture []byte

// Entry is a regular archive file, or a symbolic link when Link is nonempty.
type Entry struct {
	Path string
	Data []byte
	Link string
}

// Entries returns the small root used by image-import and manager scenarios.
func Entries() []Entry {
	return []Entry{{Path: "usr/bin/demi-runner", Data: []byte("runner")}, {Path: "usr/bin/tini", Data: []byte("tini")}, {Path: "usr/sbin/init", Link: "../bin/tini"}, {Path: "etc/skel/.profile", Data: []byte("export EDITOR=vi\n")}}
}

// CloudImage is a fixture release directory, owned by its test.
type CloudImage struct {
	Directory string
	Manifest  machinewire.CloudImageManifest
}

// NewCloudImage writes a tar/zstd release and matching generated manifest.
func NewCloudImage(t *testing.T, entries []Entry, architecture machinewire.Architecture, executables ...Entry) *CloudImage {
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
		kind := byte(tar.TypeReg)
		if entry.Link != "" {
			kind = tar.TypeSymlink
		}
		if err = writer.WriteHeader(&tar.Header{Name: entry.Path, Typeflag: kind, Linkname: entry.Link, Mode: 0755, Size: int64(len(entry.Data))}); err != nil {
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
	manifest.Rootfs.Size = uint64(compressed.Len())
	manifest.Rootfs.SHA256 = fmt.Sprintf("%x", sha256.Sum256(compressed.Bytes()))
	image := &CloudImage{Directory: t.TempDir(), Manifest: manifest}
	if err = os.WriteFile(filepath.Join(image.Directory, "rootfs.tar.zst"), compressed.Bytes(), 0600); err != nil {
		t.Fatal(err)
	}
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
	if err = os.WriteFile(filepath.Join(i.Directory, "manifest.json"), data, 0600); err != nil {
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
	if err = os.WriteFile(path, change(data), 0600); err != nil {
		t.Fatal(err)
	}
}
