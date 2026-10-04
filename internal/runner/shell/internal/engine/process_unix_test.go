//go:build darwin || linux

package engine_test

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/runner/process"
	"golang.org/x/sys/unix"
)

func TestExecEndsTheShellWithItsCommandAndLeavesTheRunner(t *testing.T) {
	result, output, stderr := shellFiles(
		t,
		t.TempDir(),
		`(exec /bin/sh -c 'exit 3'); echo "subshell $?"; exec /bin/sh -c 'echo last; exit 7'; echo after`,
		nil,
	)
	if result.Code != 7 || output != "subshell 3\nlast\n" {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
}

func TestUlimitLimitsTheJobsOwnProcessesOnly(t *testing.T) {
	var before, after unix.Rlimit
	if err := unix.Getrlimit(unix.RLIMIT_NOFILE, &before); err != nil {
		t.Fatal(err)
	}
	result, output, stderr := shellFiles(
		t,
		t.TempDir(),
		`ulimit -n 64; ulimit -n; /bin/sh -c 'ulimit -n'; ulimit -n 99999999999; echo "refused $?"; ulimit -Hn`,
		nil,
	)
	if result.Code != 0 || output != "64\n64\nrefused 1\n64\n" ||
		!strings.Contains(stderr, "open files: cannot modify limit") {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
	if err := unix.Getrlimit(unix.RLIMIT_NOFILE, &after); err != nil {
		t.Fatal(err)
	}
	if before != after {
		t.Fatalf("runner limit changed: %v -> %v", before, after)
	}
	_, output, _ = shellFiles(t, t.TempDir(), `ulimit -n; /bin/sh -c 'ulimit -n'`, nil)
	soft, _, err := process.ChildLimit(unix.RLIMIT_NOFILE)
	if err != nil {
		t.Fatal(err)
	}
	if output != fmt.Sprintf("%d\n%d\n", soft, soft) {
		t.Fatalf("another job inherited limits: %q", output)
	}
}

func TestUmaskMasksTheJobsOwnProcessesAndFilesOnly(t *testing.T) {
	root := t.TempDir()
	before := filepath.Join(root, "runner-before")
	if err := os.WriteFile(before, nil, 0o666); err != nil {
		t.Fatal(err)
	}
	result, output, stderr := shellFiles(
		t,
		root,
		`umask 077; umask; /bin/sh -c umask; umask -S; echo x > redirected; touch touched; mkdir made`,
		nil,
	)
	if result.Code != 0 || output != "0077\n0077\nu=rwx,g=,o=\n" {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
	for name, want := range map[string]os.FileMode{"redirected": 0o600, "touched": 0o600, "made": 0o700} {
		info, err := os.Stat(filepath.Join(root, name))
		if err != nil {
			t.Fatal(err)
		}
		if info.Mode().Perm() != want {
			t.Fatalf("%s mode %o want %o", name, info.Mode().Perm(), want)
		}
	}
	if err := os.WriteFile(filepath.Join(root, "runner-after"), nil, 0o666); err != nil {
		t.Fatal(err)
	}
	a, err := os.Stat(before)
	if err != nil {
		t.Fatal(err)
	}
	b, err := os.Stat(filepath.Join(root, "runner-after"))
	if err != nil {
		t.Fatal(err)
	}
	if a.Mode() != b.Mode() {
		t.Fatal("runner mask changed")
	}
	_, output, _ = shellFiles(t, t.TempDir(), `umask; /bin/sh -c umask`, nil)
	if want := fmt.Sprintf("%04o\n%04o\n", process.Umask(), process.Umask()); output != want {
		t.Fatalf("other job mask %q want %q", output, want)
	}
}

func TestKillRefusesTheRunner(t *testing.T) {
	result, output, stderr := shellFiles(
		t,
		t.TempDir(),
		`kill $$; echo "runner $?"; kill -s TERM 0; echo "group $?"`,
		nil,
	)
	if result.Code != 0 || output != "runner 1\ngroup 1\n" ||
		strings.Count(stderr, "a job cannot signal the runner it runs in") != 2 {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
}

func TestSuspendRefuses(t *testing.T) {
	result, output, stderr := shellFiles(t, t.TempDir(), `suspend -f; echo "suspend $?"`, nil)
	if result.Code != 0 || output != "suspend 1\n" ||
		!strings.Contains(stderr, "a job cannot suspend the runner it runs in") {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
}

func TestBackgroundKillDoesNotCancelParent(t *testing.T) {
	result, output, stderr := shellFiles(
		t,
		t.TempDir(),
		`while :; do :; done & id=$!; kill "$id"; wait "$id"; printf '%s\n' "$?"; echo parent`,
		nil,
	)
	if result.Code != 0 || output != "143\nparent\n" || stderr != "" {
		t.Fatalf("%+v: %q %q", result, output, stderr)
	}
}

func TestTrapsAreScopeLocalAndTimesIdentifiesRunner(t *testing.T) {
	result, output, stderr := shellFiles(
		t,
		t.TempDir(),
		`trap 'echo cleanup' INT TERM EXIT; (trap 'echo child' INT; trap -p INT); trap -p INT; false; times`,
		nil,
	)
	if result.Code != 0 || stderr != "" || !strings.Contains(output, "trap -- 'echo child' INT") ||
		!strings.Contains(output, "trap -- 'echo cleanup' INT") ||
		!strings.Contains(output, "runner:") ||
		!strings.HasSuffix(output, "cleanup\n") {
		t.Fatalf("%+v: %q %q", result, output, stderr)
	}
}

func TestExternalSignalStatusAndExecOptions(t *testing.T) {
	result, output, stderr := shellFiles(
		t,
		t.TempDir(),
		`/bin/sh -c 'kill -TERM $$'; echo "$?"; export DEMI_TEST_LEAK=wrong; `+
			`exec -ca named /bin/sh -c 'printf "%s/%s" "$0" "${DEMI_TEST_LEAK-unset}"'; `+
			`echo unreachable`,
		nil,
	)
	if result.Code != 0 || output != "143\nnamed/unset" || stderr != "" {
		t.Fatalf("%+v %q %q", result, output, stderr)
	}
}
