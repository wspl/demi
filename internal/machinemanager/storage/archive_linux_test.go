//go:build linux

package storage_test

import (
	"archive/tar"
	"errors"
	"testing"

	"github.com/wspl/demi/internal/machinemanager/storage"
	"github.com/wspl/demi/internal/machinemanager/storage/storagetest"
)

// Cost: in-process compression and local IO; no external resources.
func TestArchiveSafeEntriesAndUnsafeEntries(t *testing.T) {
	safe := []storagetest.Entry{
		{Kind: tar.TypeDir, Path: "./usr/"},
		{Kind: tar.TypeReg, Path: "./usr/bin/tini"},
		{Kind: tar.TypeSymlink, Path: "./bin", Link: "/usr/bin"},
		{Kind: tar.TypeLink, Path: "./usr/bin/init", Link: "./usr/bin/tini"},
	}
	requireStorage(t, storage.VetArchive(t.Context(), storagetest.WriteArchive(t, t.TempDir(), safe)))
	cases := []struct {
		member storagetest.Entry
		want   error
	}{
		{storagetest.Entry{Kind: tar.TypeReg, Path: "/etc/passwd"}, storage.ErrUnsafeEntry},
		{storagetest.Entry{Kind: tar.TypeReg, Path: "./../escape"}, storage.ErrUnsafeEntry},
		{storagetest.Entry{Kind: tar.TypeFifo, Path: "./run/pipe"}, storage.ErrUnsafeEntry},
		{storagetest.Entry{Kind: tar.TypeChar, Path: "./dev/null"}, storage.ErrUnsafeEntry},
		{storagetest.Entry{Kind: tar.TypeBlock, Path: "./dev/sda"}, storage.ErrUnsafeEntry},
		{
			storagetest.Entry{Kind: tar.TypeLink, Path: "./etc/shadow", Link: "../../etc/shadow"},
			storage.ErrUnsafeHardlink,
		},
		{storagetest.Entry{Kind: tar.TypeLink, Path: "./etc/shadow", Link: "/etc/shadow"}, storage.ErrUnsafeHardlink},
	}
	for _, test := range cases {
		path := storagetest.WriteArchive(t, t.TempDir(), []storagetest.Entry{test.member})
		if err := storage.VetArchive(t.Context(), path); !errors.Is(err, test.want) {
			t.Fatalf("%+v: %v, want %v", test.member, err, test.want)
		}
	}
}
