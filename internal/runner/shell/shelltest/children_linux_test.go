package shelltest_test

import (
	"os"
	"os/exec"
	"testing"

	"golang.org/x/sys/unix"
)

// The subprocess isolates Linux's process-wide adoption policy. It runs the
// same real utility scenarios with the runner, rather than init, owning the
// orphaned children. Normally below one second, plus race runtime shutdown.
func TestSubreaperCancellationReapsNativeUtilityChildren(t *testing.T) {
	const marker = "DEMI_TEST_SHELL_SUBREAPER"
	if os.Getenv(marker) == "1" {
		if err := unix.Prctl(unix.PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0); err != nil {
			t.Fatal(err)
		}
		TestCancellationReapsExternalProgramsStartedByNativeUtilities(t)
		return
	}
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	// Decision 4 leaves the GNU/BSD sed readiness mismatch outside this work.
	cmd := exec.CommandContext(t.Context(), executable,
		"-test.run=^TestSubreaperCancellationReapsNativeUtilityChildren$",
		"-test.skip=TestSubreaperCancellationReapsNativeUtilityChildren/.*sed",
		"-test.count=1", "-test.timeout=30s")
	cmd.Env = append(os.Environ(), marker+"=1")
	if output, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("subreaper scenarios: %v\n%s", err, output)
	}
}
