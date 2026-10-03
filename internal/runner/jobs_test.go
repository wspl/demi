package runner

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/runnerwire"
)

// About one second: exercises the built runner and its real pipeline processes.
func TestFunctionsAndCompoundPipelinesDrainLargeOutputAndHereDocuments(t *testing.T) {
	f := newRunner(t, nil, "")
	f.online()
	if err := os.WriteFile(filepath.Join(f.home, "input"), []byte(strings.Repeat("x", 262144)), 0600); err != nil {
		t.Fatal(err)
	}
	f.job("pipeline", "producer() { cat input; }; value=$(producer | cat | cat); printf '%s\\n' \"${#value}\"; { producer; } | wc -c; (producer) | wc -c; cat <<EOF | wc -c\n$value\nEOF\ncat <<< \"$value\" | wc -c")
	out, stderr, exit := f.jobOutput("pipeline")
	requireJobSuccess(t, exit, stderr)
	if stderr != "" {
		t.Fatalf("unexpected stderr: %q", stderr)
	}
	if got := strings.Join(strings.Fields(out), " "); got != "262144 262144 262144 262145 262145" {
		t.Fatalf("pipeline: %q, stderr: %q", out, stderr)
	}
}
func TestCancellationTerminatesBlockingUtility(t *testing.T) {
	f := newRunner(t, nil, "")
	f.online()
	f.job("job", "printf ready; sleep 60")
	if _, ok := f.frame().(*runnerwire.JobOutput); !ok {
		t.Fatal("utility did not start")
	}
	f.send(&runnerwire.JobKill{JobID: "job", Signal: new(runnerwire.SignalKill)})
	_, _, exit := f.jobOutput("job")
	if exit.Signal == nil || *exit.Signal != "SIGKILL" {
		t.Fatalf("cancelled job: %+v", exit)
	}
}
