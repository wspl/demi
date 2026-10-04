package runner

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// About one second: exercises the built runner and its real pipeline processes.
func TestFunctionsAndCompoundPipelinesDrainLargeOutputAndHereDocuments(t *testing.T) {
	f := newRunner(t, nil, "")
	f.online()
	if err := os.WriteFile(filepath.Join(f.home, "input"), []byte(strings.Repeat("x", 262144)), 0o600); err != nil {
		t.Fatal(err)
	}
	f.job(
		"pipeline",
		"producer() { cat input; }; value=$(producer | cat | cat); printf '%s\\n' \"${#value}\"; "+
			"{ producer; } | wc -c; (producer) | wc -c; cat <<EOF | wc -c\n$value\nEOF\ncat <<< \"$value\" | wc -c",
	)
	out, stderr, exit := f.jobOutput("pipeline")
	requireJobSuccess(t, exit, stderr)
	if stderr != "" {
		t.Fatalf("unexpected stderr: %q", stderr)
	}
	if got := strings.Join(strings.Fields(out), " "); got != "262144 262144 262144 262145 262145" {
		t.Fatalf("pipeline: %q, stderr: %q", out, stderr)
	}
}
