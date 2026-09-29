//go:build linux

package shell_test

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"testing"
	"time"

	"github.com/wspl/demi/go/runner/shell"
)

// An open that finds the descriptor table full waits until a descriptor
// closes (runner.md § Load): the job's null input, and the login profile,
// which is then read instead of taken for missing.
// Cost: one child test process under a 64-descriptor limit; its waits are for
// the shell to block on the full table and for the job's result, bounded by a
// ten-second guard each.
func TestFullDescriptorTableWaitsForNullInputAndProfile(t *testing.T) {
	if os.Getenv("DEMI_TEST_FULL_TABLE") != "1" {
		executable, err := os.Executable()
		if err != nil {
			t.Fatal(err)
		}
		cmd := exec.CommandContext(t.Context(), "/bin/sh", "-c", `ulimit -Sn 64; exec "$1" -test.run '^TestFullDescriptorTableWaitsForNullInputAndProfile$'`, "full-table-test", executable)
		cmd.Env = append(os.Environ(), "DEMI_TEST_FULL_TABLE=1")
		if output, err := cmd.CombinedOutput(); err != nil {
			t.Fatalf("full-table shell: %v\n%s", err, output)
		}
		return
	}
	// Without login the null input is the first open; with login the job gets
	// an input, so the profile's is.
	for _, login := range []bool{false, true} {
		opts := options(t)
		opts.Login = login
		if err := os.WriteFile(filepath.Join(opts.Dir, ".bash_profile"), []byte("export FROM_PROFILE=yes\n"), 0600); err != nil {
			t.Fatal(err)
		}
		if login {
			input, err := os.Open(os.DevNull)
			if err != nil {
				t.Fatal(err)
			}
			opts.Stdin = input
		}
		out := &output{}
		opts.Output = out
		guard, stop := context.WithTimeout(t.Context(), 10*time.Second)
		fillers := fillTable(t)
		done := make(chan shell.Result, 1)
		go func() { done <- shell.Run(guard, `printf '%s' "$FROM_PROFILE"`, opts) }()
		waitRetrying(t, guard, done)
		for _, file := range fillers {
			file.Close()
		}
		var result shell.Result
		select {
		case result = <-done:
		case <-guard.Done():
			t.Fatal("the job did not finish once descriptors closed")
		}
		stop()
		want := ""
		if login {
			want = "yes"
		}
		if result.Err != nil || result.Code == nil || *result.Code != 0 || out.stdout.String() != want {
			t.Fatalf("login=%v: %+v stdout=%q stderr=%q", login, result, out.stdout.String(), out.stderr.String())
		}
	}
}

// fillTable opens /dev/null until the process has no descriptor left.
func fillTable(t *testing.T) []*os.File {
	t.Helper()
	var files []*os.File
	for {
		file, err := os.Open(os.DevNull)
		if errors.Is(err, syscall.EMFILE) {
			return files
		}
		if err != nil {
			t.Fatal(err)
		}
		files = append(files, file)
	}
}

// waitRetrying waits until the job is waiting out the full table, which no
// event marks: it polls the goroutine dump for the retry loop. A job that ends
// first failed instead of waiting.
func waitRetrying(t *testing.T, ctx context.Context, done <-chan shell.Result) {
	t.Helper()
	dump := make([]byte, 1<<20)
	for {
		if strings.Contains(string(dump[:runtime.Stack(dump, true)]), "commandservice.RetryBlocking") {
			return
		}
		select {
		case result := <-done:
			t.Fatalf("the job ended instead of waiting for a descriptor: %+v", result)
		case <-ctx.Done():
			t.Fatal("the job did not wait for a descriptor")
		case <-time.After(time.Millisecond):
		}
	}
}
