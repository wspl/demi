package artifacts_test

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/wspl/demi/internal/artifacts"
)

// Cost: local filesystem operations only, no subprocesses or timed waits.
func TestPublicationStagesBesideSymlinkParent(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("symlink creation requires Windows privileges")
	}
	root := t.TempDir()
	other := t.TempDir()
	must(t, os.Mkdir(filepath.Join(other, "dir"), 0700))
	must(t, os.Symlink(filepath.Join(other, "dir"), filepath.Join(root, "link")))
	destination := root + "/link/../file"
	staged, err := artifacts.NewStaged(t.Context(), destination, artifacts.Publication{Durable: true})
	must(t, err)
	defer func() { must(t, staged.Close()) }()
	if got := names(t, other); len(got) != 2 {
		t.Fatalf("stage must be in actual parent: %v", got)
	}
	_, err = staged.File().Write(body)
	must(t, err)
	must(t, staged.Publish(t.Context()))
	contents(t, filepath.Join(other, "file"), body)
}

// Cost: pure path parsing.
func TestParent(t *testing.T) {
	cases := []struct {
		path, parent string
		ok           bool
	}{
		{"", "", false}, {"/", "", false}, {"file", "", true},
		{".", "", true}, {"..", "", true}, {"./file", ".", true},
		{"a/../file", "a/..", true}, {"a/./file", "a", true},
		{"a//b///./", "a", true}, {"/a", "/", true},
		{"/a/link/../file", "/a/link/..", true},
	}
	if runtime.GOOS == "windows" {
		cases = append(cases, struct {
			path, parent string
			ok           bool
		}{`C:\a\..\file`, `C:\a\..`, true})
		cases = append(cases, struct {
			path, parent string
			ok           bool
		}{`C:\`, "", false})
		cases = append(cases, struct {
			path, parent string
			ok           bool
		}{`\\server\share\file`, `\\server\share\`, true})
	}
	for _, tc := range cases {
		parent, ok := artifacts.Parent(tc.path)
		if parent != tc.parent || ok != tc.ok {
			t.Errorf("Parent(%q) = %q, %v; want %q, %v", tc.path, parent, ok, tc.parent, tc.ok)
		}
	}
}
