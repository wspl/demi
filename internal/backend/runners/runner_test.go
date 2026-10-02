package runners_test

import (
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/host"
)

// One real runner covers listings and text reads in under ten seconds.
// remotehosttest/programtest builds the runner used by the default suite.
func TestRunnerFileListingsAndText(t *testing.T) {
	fixture, err := remotehosttest.StartRunnerFixture(t.Context(), t, remotehosttest.FixtureOptions{})
	if err != nil {
		t.Fatal(err)
	}
	fs := fixture.Host().FS()
	path := filepath.Join(fixture.Home(), "report.txt")
	if err := fs.WriteFile(t.Context(), path, host.FileContents{Bytes: []byte("first\nsecond\n")}, host.WriteOptions{}); err != nil {
		t.Fatal(err)
	}
	text, err := runners.ReadTextFile(t.Context(), fs, path)
	if err != nil || text != "first\nsecond\n" {
		t.Fatalf("text %q: %v", text, err)
	}
	entries, err := runners.BrowseDirectory(t.Context(), fs, fixture.Home())
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, entry := range entries {
		if entry.Name == "report.txt" {
			found = true
			if entry.Size != 13 || entry.IsDirectory || entry.IsSymbolicLink {
				t.Fatalf("entry %+v", entry)
			}
		}
	}
	if !found {
		t.Fatal("report absent from directory")
	}
	if err := fs.WriteFile(t.Context(), path, host.FileContents{Bytes: []byte{0, 255}}, host.WriteOptions{}); err != nil {
		t.Fatal(err)
	}
	if _, err := runners.ReadTextFile(t.Context(), fs, path); !errors.Is(err, runners.TextNotText) {
		t.Fatalf("binary file: %v", err)
	}
	// A sparse file exercises the metadata limit without transferring its bytes.
	large, err := os.Create(filepath.Join(fixture.Home(), "large"))
	if err != nil {
		t.Fatal(err)
	}
	if err := large.Truncate(commandwire.EditFileBytes + 1); err != nil {
		_ = large.Close()
		t.Fatal(err)
	}
	if err := large.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := runners.ReadTextFile(t.Context(), fs, filepath.Join(fixture.Home(), "large")); !errors.Is(err, runners.TextTooLarge) {
		t.Fatalf("large file: %v", err)
	}
}
