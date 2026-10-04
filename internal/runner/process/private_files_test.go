package process_test

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/wspl/demi/internal/runner/process"
)

func TestPrivatePublicationAndPermissions(t *testing.T) {
	path := filepath.Join(t.TempDir(), "state")
	for _, data := range []string{"first", "replacement\n"} {
		if err := process.WritePrivate(t.Context(), path, []byte(data)); err != nil {
			t.Fatal(err)
		}
		contents, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		want := data
		if want[len(want)-1] != '\n' {
			want += "\n"
		}
		if string(contents) != want {
			t.Fatalf("contents %q, want %q", contents, want)
		}
		if runtime.GOOS != "windows" {
			info, err := os.Stat(path)
			if err != nil {
				t.Fatal(err)
			}
			if info.Mode().Perm() != 0o600 {
				t.Fatalf("publication mode %o", info.Mode().Perm())
			}
		}
	}
	if err := process.Chmod(t.Context(), path, 0o640); err != nil {
		t.Fatal(err)
	}
	if runtime.GOOS != "windows" {
		info, err := os.Stat(path)
		if err != nil {
			t.Fatal(err)
		}
		if info.Mode().Perm() != 0o640 {
			t.Fatalf("mode %o", info.Mode().Perm())
		}
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if err := process.WritePrivate(ctx, path, []byte("cancelled")); !errors.Is(err, context.Canceled) {
		t.Fatalf("cancelled publication: %v", err)
	}
	data, err := os.ReadFile(path)
	if err != nil || string(data) != "replacement\n" {
		t.Fatalf("cancelled publication replaced state: %q %v", data, err)
	}
	entries, err := os.ReadDir(filepath.Dir(path))
	if err != nil || len(entries) != 1 {
		t.Fatalf("leftover files: %v %v", entries, err)
	}
}
