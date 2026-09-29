//go:build unix

package shell_test

import (
	"os"
	"os/exec"
	"strings"
	"testing"
)

// One isolated test process, a native probe, and two comparison children. No
// process-wide limit is changed in the parent test suite; waits are for exits.
func TestInheritedOpenFileLimits(t *testing.T) {
	if os.Getenv("DEMI_TEST_LOW_NOFILE") != "1" {
		executable, err := os.Executable()
		if err != nil {
			t.Fatal(err)
		}
		cmd := exec.CommandContext(t.Context(), "/bin/sh", "-c", `ulimit -Sn 128; exec "$1" -test.run '^TestInheritedOpenFileLimits$'`, "limits-test", executable)
		cmd.Env = append(os.Environ(), "DEMI_TEST_LOW_NOFILE=1")
		if output, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("low-limit shell: %v\n%s", err, output)
		}
		return
	}
	opts := options(t)
	result, out := execute(t, opts, `ulimit -Sn; ulimit -Hn; /bin/sh -c 'ulimit -Sn; ulimit -Hn'; ulimit -Sn 64; ulimit -Sn; /bin/sh -c 'ulimit -Sn'`)
	fields := strings.Fields(out.stdout.String())
	if *result.Code != 0 || len(fields) != 6 || fields[0] != "128" || fields[2] != "128" || fields[1] != fields[3] || fields[4] != "64" || fields[5] != "64" {
		t.Fatalf("builtin and child limits: status=%d stdout=%q stderr=%q", *result.Code, out.stdout.String(), out.stderr.String())
	}
	// A fresh job keeps the discovered baseline, not the previous job's change.
	_, out = execute(t, opts, "ulimit -Sn")
	if out.stdout.String() != "128\n" {
		t.Fatalf("fresh job limit: %q", out.stdout.String())
	}
}
