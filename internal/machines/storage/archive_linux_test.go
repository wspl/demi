//go:build linux

package storage_test

import (
	"archive/tar"
	"bytes"
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/klauspost/compress/zstd"
	"github.com/wspl/demi/internal/machines/storage"
)

type archiveMember struct {
	kind       byte
	name, link string
	data       []byte
}

// cloudArchive creates the same small tar/zstd release inputs as Rust's fixtures.
func cloudArchive(t *testing.T, directory string, members []archiveMember) string {
	t.Helper()
	var data bytes.Buffer
	encoder, err := zstd.NewWriter(&data, zstd.WithEncoderConcurrency(1))
	requireStorage(t, err)
	writer := tar.NewWriter(encoder)
	for _, entry := range members {
		requireStorage(t, writer.WriteHeader(&tar.Header{Typeflag: entry.kind, Name: entry.name, Linkname: entry.link, Mode: 0755, Size: int64(len(entry.data))}))
		_, err := writer.Write(entry.data)
		requireStorage(t, err)
	}
	requireStorage(t, writer.Close())
	requireStorage(t, encoder.Close())
	path := filepath.Join(directory, "rootfs.tar.zst")
	requireStorage(t, os.WriteFile(path, data.Bytes(), 0600))
	return path
}

// Cost: in-process compression and local IO; no external resources.
func TestArchiveSafeEntriesAndUnsafeEntries(t *testing.T) {
	safe := []archiveMember{
		{kind: tar.TypeDir, name: "./usr/"},
		{kind: tar.TypeReg, name: "./usr/bin/tini"},
		{kind: tar.TypeSymlink, name: "./bin", link: "/usr/bin"},
		{kind: tar.TypeLink, name: "./usr/bin/init", link: "./usr/bin/tini"},
	}
	requireStorage(t, storage.VetArchive(t.Context(), cloudArchive(t, t.TempDir(), safe)))
	cases := []struct {
		member archiveMember
		want   error
	}{
		{archiveMember{kind: tar.TypeReg, name: "/etc/passwd"}, storage.ErrUnsafeEntry},
		{archiveMember{kind: tar.TypeReg, name: "./../escape"}, storage.ErrUnsafeEntry},
		{archiveMember{kind: tar.TypeFifo, name: "./run/pipe"}, storage.ErrUnsafeEntry},
		{archiveMember{kind: tar.TypeChar, name: "./dev/null"}, storage.ErrUnsafeEntry},
		{archiveMember{kind: tar.TypeBlock, name: "./dev/sda"}, storage.ErrUnsafeEntry},
		{archiveMember{kind: tar.TypeLink, name: "./etc/shadow", link: "../../etc/shadow"}, storage.ErrUnsafeHardlink},
		{archiveMember{kind: tar.TypeLink, name: "./etc/shadow", link: "/etc/shadow"}, storage.ErrUnsafeHardlink},
	}
	for _, test := range cases {
		path := cloudArchive(t, t.TempDir(), []archiveMember{test.member})
		if err := storage.VetArchive(t.Context(), path); !errors.Is(err, test.want) {
			t.Fatalf("%+v: %v, want %v", test.member, err, test.want)
		}
	}
}
