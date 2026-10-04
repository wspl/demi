package programtest_test

import (
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"testing"

	"github.com/wspl/demi/internal/programtest"
)

// A build would return a path outside the supplied directory for "fixture"
// and a build error rather than os.ErrNotExist for "missing".
// Cost: one temporary file, no build; well under one second.
func TestSuppliedProgramsNeverBuild(t *testing.T) {
	directory := t.TempDir()
	t.Setenv("DEMI_TEST_PROGRAMS", directory)
	name := "fixture"
	if runtime.GOOS == "windows" {
		name += ".exe"
	}
	expected := filepath.Join(directory, name)
	if err := os.WriteFile(expected, []byte("supplied"), 0o600); err != nil {
		t.Fatal(err)
	}
	actual, err := programtest.Path(t.Context(), "fixture")
	if err != nil || actual != expected {
		t.Fatalf("Path = %q, %v; want %q", actual, err, expected)
	}
	if _, err := programtest.Path(t.Context(), "missing"); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("missing supplied program: %v", err)
	}
}
