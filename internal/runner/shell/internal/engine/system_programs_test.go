package engine_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// A system program runs in the invocation's directory, and its exit status and
// diagnostics belong to that invocation alone.
func TestUtilitiesCwdAndExitStateArePerInvocation(t *testing.T) {
	first, second := t.TempDir(), t.TempDir()
	for root, value := range map[string]string{first: "one", second: "two"} {
		if err := os.WriteFile(filepath.Join(root, "file"), []byte(value), 0o600); err != nil {
			t.Fatal(err)
		}
		result, output, stderr := shellFiles(t, root, "cat file", nil)
		if result.Code != 0 || output != value {
			t.Fatalf("exit %d output %q stderr %q", result.Code, output, stderr)
		}
	}
	result, _, stderr := shellFiles(t, first, "cat missing", nil)
	if result.Code != 1 || !strings.HasPrefix(stderr, "cat:") {
		t.Fatalf("failure %d: %s", result.Code, stderr)
	}
	result, output, stderr := shellFiles(t, second, "cat file", nil)
	if result.Code != 0 || output != "two" {
		t.Fatalf("failure leaked: %d %q %q", result.Code, output, stderr)
	}
}
