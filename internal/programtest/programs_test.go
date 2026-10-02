package programtest

import (
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

func TestSuppliedProgramsNeverBuild(t *testing.T) {
	directory := t.TempDir()
	t.Setenv("DEMI_TEST_PROGRAMS", directory)
	name := "fixture"
	if runtime.GOOS == "windows" {
		name += ".exe"
	}
	expected := filepath.Join(directory, name)
	if err := os.WriteFile(expected, []byte("supplied"), 0600); err != nil {
		t.Fatal(err)
	}
	actual, err := Path(t.Context(), "fixture")
	if err != nil || actual != expected {
		t.Fatalf("Path = %q, %v; want %q", actual, err, expected)
	}
	if _, err := Path(t.Context(), "missing"); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("missing supplied program: %v", err)
	}
	if len(programs.builds) != 0 {
		t.Fatal("supplied programs must not build")
	}
}
