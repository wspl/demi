//go:build darwin || linux

package shell

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
	result, output, stderr := shellFiles(t, t.TempDir(), `(exec /bin/sh -c 'exit 3'); echo "subshell $?"; exec /bin/sh -c 'echo last; exit 7'; echo after`, nil)
	if result.code != 7 || output != "subshell 3\nlast\n" {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
}
func TestUlimitLimitsTheJobsOwnProcessesOnly(t *testing.T) {
	var before, after unix.Rlimit
	if err := unix.Getrlimit(unix.RLIMIT_NOFILE, &before); err != nil {
		t.Fatal(err)
	}
	result, output, stderr := shellFiles(t, t.TempDir(), `ulimit -n 64; ulimit -n; /bin/sh -c 'ulimit -n'; ulimit -n 99999999999; echo "refused $?"; ulimit -Hn`, nil)
	if result.code != 0 || output != "64\n64\nrefused 1\n64\n" || !strings.Contains(stderr, "open files: cannot modify limit") {
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
	if err := os.WriteFile(before, nil, 0666); err != nil {
		t.Fatal(err)
	}
	result, output, stderr := shellFiles(t, root, `umask 077; umask; /bin/sh -c umask; umask -S; echo x > redirected; touch touched; mkdir made`, nil)
	if result.code != 0 || output != "0077\n0077\nu=rwx,g=,o=\n" {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
	for name, want := range map[string]os.FileMode{"redirected": 0600, "touched": 0600, "made": 0700} {
		info, err := os.Stat(filepath.Join(root, name))
		if err != nil {
			t.Fatal(err)
		}
		if info.Mode().Perm() != want {
			t.Fatalf("%s mode %o want %o", name, info.Mode().Perm(), want)
		}
	}
	if err := os.WriteFile(filepath.Join(root, "runner-after"), nil, 0666); err != nil {
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
	result, output, stderr := shellFiles(t, t.TempDir(), `kill $$; echo "runner $?"; kill -s TERM 0; echo "group $?"`, nil)
	if result.code != 0 || output != "runner 1\ngroup 1\n" || strings.Count(stderr, "a job cannot signal the runner it runs in") != 2 {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
}
func TestSuspendRefuses(t *testing.T) {
	result, output, stderr := shellFiles(t, t.TempDir(), `suspend -f; echo "suspend $?"`, nil)
	if result.code != 0 || output != "suspend 1\n" || !strings.Contains(stderr, "a job cannot suspend the runner it runs in") {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
}
