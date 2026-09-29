// Package backendtest is the backend's black-box suite: it runs demi-backend as
// a process on a free port with a data directory of its own, speaks HTTP and
// WebSocket to it, pairs real demi-runner processes, and drives the hooks of a
// test build through the control socket
// (docs/internal/go-migration/design/g6-api-suite.md). The scenarios are the
// package's tests; this file and its neighbours are the harness they share.
//
// The programs come from the directory DEMI_TEST_PROGRAMS names, as in the
// TypeScript suites: demi-backend (a test build), demi-runner, and the native
// packages' executables, demi-commands, demi-claude and demi-native-fixture.
package backendtest

import (
	"os"
	"path/filepath"
	"testing"
)

// ProgramsVariable names the directory of the built programs.
const ProgramsVariable = "DEMI_TEST_PROGRAMS"

// programsDir returns the directory the programs are built in, or skips the
// test when nothing names it: a suite that needs programs is skipped where none
// are built.
func programsDir(t testing.TB) string {
	t.Helper()
	dir := os.Getenv(ProgramsVariable)
	if dir == "" {
		t.Skipf("%s names the directory of the built programs; the suite is skipped without it", ProgramsVariable)
	}
	if filepath.IsAbs(dir) {
		return dir
	}
	// A relative directory, such as target/debug, is the repository's: the one
	// of the nearest ancestor of the working directory that holds it.
	working, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	for parent := working; ; parent = filepath.Dir(parent) {
		candidate := filepath.Join(parent, dir)
		if info, err := os.Stat(candidate); err == nil && info.IsDir() {
			return candidate
		}
		if filepath.Dir(parent) == parent {
			t.Fatalf("%s=%s is no directory above %s", ProgramsVariable, dir, working)
		}
	}
}

// Program returns the path of the built program name, or fails the test when
// it is not built.
func Program(t testing.TB, name string) string {
	t.Helper()
	path := filepath.Join(programsDir(t), name)
	if _, err := os.Stat(path); err != nil {
		t.Fatalf("the program %s is not built: %v", name, err)
	}
	return path
}
