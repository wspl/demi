//go:build linux

package sandbox_test

import (
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/machinemanager/sandbox"
)

func TestRemovalPreservesMountPointContents(t *testing.T) {
	directory := sandbox.NewRuntimeDirectory(t.TempDir(), "demi-test")
	ctx := t.Context()
	if err := directory.Create(ctx); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(directory.Config(), []byte("{}"), 0o600); err != nil {
		t.Fatal(err)
	}
	project := filepath.Join(directory.Home(), "project")
	if err := os.WriteFile(project, []byte("work"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := directory.Remove(ctx); err == nil {
		t.Fatal("removed nonempty mount point")
	}
	if data, err := os.ReadFile(project); err != nil || string(data) != "work" {
		t.Fatalf("project = %q, %v", data, err)
	}
	if err := os.Remove(project); err != nil {
		t.Fatal(err)
	}
	if err := directory.Remove(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(directory.Root()); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("directory remains: %v", err)
	}
	if err := directory.Remove(ctx); err != nil {
		t.Fatal(err)
	}
}
