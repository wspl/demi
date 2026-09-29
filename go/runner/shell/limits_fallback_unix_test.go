//go:build unix

package shell

import (
	"bytes"
	"log/slog"
	"path/filepath"
	"strings"
	"testing"

	"golang.org/x/sys/unix"
)

// A failed native start must retain useful limits and explain the fallback in
// the Host log. This costs one failed start and does not mutate cached limits.
func TestInheritedLimitProbeFallback(t *testing.T) {
	var log bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewTextHandler(&log, nil)))
	defer slog.SetDefault(previous)
	var want unix.Rlimit
	if err := unix.Getrlimit(unix.RLIMIT_NOFILE, &want); err != nil {
		t.Fatal(err)
	}
	missing := filepath.Join(t.TempDir(), "missing-shell")
	got, err := readInheritedOpenFiles(t.Context(), missing)
	if err != nil || got != want {
		t.Fatalf("fallback=%+v want=%+v err=%v", got, want, err)
	}
	if !strings.Contains(log.String(), "using runner limits") || !strings.Contains(log.String(), missing) {
		t.Fatalf("fallback reason missing from Host log: %s", log.String())
	}
}
