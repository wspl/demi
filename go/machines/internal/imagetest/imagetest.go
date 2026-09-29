// Package imagetest makes Cloud image releases for the machine manager's tests
// (docs/cloud/images.md § Release artifacts): a release directory with its
// archive and manifest. The manager's unit tests and its process tests use them.
package imagetest

import (
	"archive/tar"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json/v2"
	"os"
	"path/filepath"
	"testing"

	"github.com/klauspost/compress/zstd"

	"github.com/wspl/demi/go/machinesproto"
)

// An Entry is an archive entry: a regular file's contents, or a symbolic link's
// target.
type Entry struct {
	Path string
	Body []byte
	Link string
}

// An Executable is an embedded executable: its path in the image and its
// contents.
type Executable struct {
	Path string
	Body []byte
}

// The runner and the init every image embeds.
var (
	Runner = Executable{Path: "/usr/bin/demi-runner", Body: []byte("runner")}
	Tini   = Executable{Path: "/usr/bin/tini", Body: []byte("tini")}
)

// Entries returns the entries of a small root: the runner, tini as init and a
// skeleton profile.
func Entries() []Entry {
	return []Entry{
		{Path: "usr/bin/demi-runner", Body: Runner.Body},
		{Path: "usr/bin/tini", Body: Tini.Body},
		{Path: "usr/sbin/init", Link: "../bin/tini"},
		{Path: "etc/skel/.profile", Body: []byte("export EDITOR=vi\n")},
	}
}

// Digest returns the SHA-256 of bytes in hexadecimal, as manifests write it.
func Digest(bytes []byte) string {
	sum := sha256.Sum256(bytes)
	return hex.EncodeToString(sum[:])
}

// An Image is a release directory whose archive holds the given entries and whose
// manifest lists the given executables.
type Image struct {
	// Dir is the release directory.
	Dir string
}

// New writes a release directory of architecture, removed with the test.
func New(t testing.TB, entries []Entry, executables []Executable, architecture machinesproto.Architecture) *Image {
	t.Helper()
	directory := t.TempDir()
	archive := filepath.Join(directory, "rootfs.tar.zst")
	file, err := os.Create(archive)
	if err != nil {
		t.Fatal(err)
	}
	encoder, err := zstd.NewWriter(file)
	if err != nil {
		t.Fatal(err)
	}
	writer := tar.NewWriter(encoder)
	for _, entry := range entries {
		header := &tar.Header{Name: entry.Path, Mode: 0o755, Typeflag: tar.TypeReg, Size: int64(len(entry.Body))}
		if entry.Link != "" {
			header = &tar.Header{Name: entry.Path, Typeflag: tar.TypeSymlink, Linkname: entry.Link}
		}
		if err := writer.WriteHeader(header); err != nil {
			t.Fatal(err)
		}
		if _, err := writer.Write(entry.Body); err != nil {
			t.Fatal(err)
		}
	}
	for _, closer := range []interface{ Close() error }{writer, encoder, file} {
		if err := closer.Close(); err != nil {
			t.Fatal(err)
		}
	}
	bytes, err := os.ReadFile(archive)
	if err != nil {
		t.Fatal(err)
	}
	artifact := func(body []byte) map[string]any {
		return map[string]any{"sha256": Digest(body), "size": len(body)}
	}
	embedded := map[string]any{}
	for _, executable := range executables {
		embedded[executable.Path] = artifact(executable.Body)
	}
	manifest := map[string]any{
		"formatVersion": 1,
		"os":            "linux",
		"architecture":  string(architecture),
		"rootfs":        map[string]any{"sha256": Digest(bytes), "size": len(bytes), "file": "rootfs.tar.zst"},
		"ubuntu":        "26.04",
		"packages":      []any{},
		"executables":   embedded,
		"releases":      []any{},
		"runner": map[string]any{
			"release":         "f" + Digest(nil)[1:],
			"wire":            24,
			"commandProtocol": 1,
			"targets":         map[string]any{string(architecture.Target()): artifact(Runner.Body)},
		},
		"tools": []any{},
	}
	image := &Image{Dir: directory}
	image.write(t, manifest)
	return image
}

func (i *Image) write(t testing.TB, manifest map[string]any) {
	t.Helper()
	data, err := json.Marshal(manifest, json.Deterministic(true))
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(i.Dir, "manifest.json"), data, 0o644); err != nil {
		t.Fatal(err)
	}
}

// Edit changes the manifest, for a release that fails a check.
func (i *Image) Edit(t testing.TB, change func(manifest map[string]any)) {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(i.Dir, "manifest.json"))
	if err != nil {
		t.Fatal(err)
	}
	var manifest map[string]any
	if err := json.Unmarshal(data, &manifest); err != nil {
		t.Fatal(err)
	}
	change(manifest)
	i.write(t, manifest)
}
