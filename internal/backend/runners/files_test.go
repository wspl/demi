package runners_test

import (
	"bytes"
	"context"
	"errors"
	"testing"

	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/host"
)

func TestTextRefusals(t *testing.T) {
	for _, tc := range []struct {
		data []byte
		text string
		err  error
	}{
		{[]byte("1\n2\n"), "1\n2\n", nil},
		{[]byte{0, 255, 1}, "", runners.ErrTextNotText},
		{[]byte("nul \x00 inside"), "", runners.ErrTextNotText},
		{bytes.Repeat([]byte("a"), commandwire.EditFileBytes+1), "", runners.ErrTextTooLarge},
	} {
		got, err := runners.TextOf(tc.data)
		if got != tc.text || !errors.Is(err, tc.err) {
			t.Fatalf("text len=%d: %q %v", len(tc.data), got, err)
		}
	}
}

// listedFS scripts the Host listing boundary; unexpected FS calls are test failures.
type listedFS struct {
	host.FS
	reads int
	size  uint64
}

func (f *listedFS) ReadDir(context.Context, string) ([]host.DirEntry, error) {
	return []host.DirEntry{
		{Name: "file", Kind: host.File},
		{Name: "gone", Kind: host.File},
		{Name: "link", Kind: host.Directory},
	}, nil
}

func (f *listedFS) Lstat(_ context.Context, path string) (host.FileStat, error) {
	switch path {
	case "/work/file":
		return host.FileStat{Kind: host.File, Size: 3}, nil
	case "/work/gone":
		return host.FileStat{}, &host.Error{Kind: host.Failed, Code: "ENOENT"}
	default:
		return host.FileStat{Kind: host.Symlink, Size: 4}, nil
	}
}

func (f *listedFS) Stat(context.Context, string) (host.FileStat, error) {
	return host.FileStat{Size: f.size}, nil
}

func (f *listedFS) ReadFile(context.Context, string) ([]byte, error) {
	f.reads++
	return []byte("hello"), nil
}

func TestHostFileListingAndEarlySizeRefusal(t *testing.T) {
	fs := &listedFS{size: commandwire.EditFileBytes + 1}
	if _, err := runners.ReadTextFile(
		t.Context(),
		fs,
		"/work/file",
	); !errors.Is(err, runners.ErrTextTooLarge) ||
		fs.reads != 0 {
		t.Fatalf("oversized file was read: %v, %d reads", err, fs.reads)
	}
	entries, err := runners.BrowseDirectory(t.Context(), fs, "/work///")
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 2 || entries[0].Name != "file" || entries[1].Name != "link" ||
		!entries[1].IsDirectory || !entries[1].IsSymbolicLink {
		t.Fatalf("listing: %+v", entries)
	}
	fs.size = 5
	text, err := runners.ReadTextFile(t.Context(), fs, "/work/file")
	if err != nil || text != "hello" {
		t.Fatalf("text: %q %v", text, err)
	}
}
