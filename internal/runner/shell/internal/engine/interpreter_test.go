package engine

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/runner/process"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) {
	if handled, err := process.RunChildBootstrap(); handled {
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		os.Exit(0)
	}
	goleak.VerifyTestMain(m)
}

// shellFiles owns one script's files and reads them only after execution has joined.
func shellFiles(t *testing.T, root, script string, configure func(*Options)) (Result, string, string) {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), 10*time.Second)
	defer cancel()
	files := make([]*os.File, 3)
	for index := range files {
		file, err := os.CreateTemp(t.TempDir(), "shell")
		if err != nil {
			t.Fatal(err)
		}
		files[index] = file
		defer func() { _ = file.Close() }() // Cleanup also runs after cancellation closes the file.
	}
	options := Options{Cwd: root, Env: map[string]string{"HOME": root, "PATH": os.Getenv("PATH"), "TMPDIR": root}, Stdin: files[0], Stdout: files[1], Stderr: files[2]}
	if configure != nil {
		configure(&options)
	}
	result, err := Execute(ctx, script, options)
	if err != nil {
		t.Fatal(err)
	}
	stdout, err := os.ReadFile(files[1].Name())
	if err != nil {
		t.Fatal(err)
	}
	stderr, err := os.ReadFile(files[2].Name())
	if err != nil {
		t.Fatal(err)
	}
	return result, string(stdout), string(stderr)
}

func TestShellHandlesRedirectsFunctionsSubshellCwdAndFreshState(t *testing.T) {
	root := t.TempDir()
	result, output, stderr := shellFiles(t, root, `mkdir a b; f() { printf '%s\n' "$1"; }; f pear > a/input; (cd a; cat input) | tr a-z A-Z; cd b; export LEAK=bad`, nil)
	if result.Code != 0 || result.Cwd != filepath.Join(root, "b") || output != "PEAR\n" {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
	_, output, _ = shellFiles(t, result.Cwd, `printf '%s' "${LEAK-unset}"`, nil)
	if output != "unset" {
		t.Fatalf("state leaked: %q", output)
	}
}

func TestTeeAndOdUsePipelineStreams(t *testing.T) {
	root := t.TempDir()
	result, output, stderr := shellFiles(t, root, `printf hello | tee made.txt | grep hello | od -An -tx1`, nil)
	data, err := os.ReadFile(filepath.Join(root, "made.txt"))
	if err != nil || string(data) != "hello" || result.Code != 0 || strings.Join(strings.Fields(output), " ") != "68 65 6c 6c 6f 0a" {
		t.Fatalf("file %q (%v), result %+v output %q stderr %q", data, err, result, output, stderr)
	}
}

func TestWcCountsAFileOfWholePagesWhole(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "pages.bin"), make([]byte, 5*65536), 0600); err != nil {
		t.Fatal(err)
	}
	result, output, stderr := shellFiles(t, root, "wc -c pages.bin", nil)
	if result.Code != 0 || strings.Join(strings.Fields(output), " ") != "327680 pages.bin" {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
}

func TestLoginProfilesApplyPerJobWithoutReplacingOwnedContextOrCwd(t *testing.T) {
	root := t.TempDir()
	if err := os.Mkdir(filepath.Join(root, "elsewhere"), 0700); err != nil {
		t.Fatal(err)
	}
	for _, value := range []string{"first", "next"} {
		profile := "export FROM_PROFILE=" + value + "\nexport PATH=\"$HOME/tools\"\nexport DEMI_CONTEXT_ID=wrong\ncd \"$HOME/elsewhere\"\n"
		if err := os.WriteFile(filepath.Join(root, ".bash_profile"), []byte(profile), 0600); err != nil {
			t.Fatal(err)
		}
		result, output, stderr := shellFiles(t, root, `printf '%s\n' "$FROM_PROFILE" "$DEMI_CONTEXT_ID" "$PATH"`, func(options *Options) {
			options.Login = true
			options.Env["DEMI_CONTEXT_ID"] = "owned"
			options.Env["PATH"] = filepath.Join(root, "aliases")
		})
		want := value + "\nowned\n" + filepath.Join(root, "aliases") + string(os.PathListSeparator) + filepath.Join(root, "tools") + "\n"
		if result.Code != 0 || result.Cwd != root || output != want {
			t.Fatalf("result %+v output %q want %q stderr %q", result, output, want, stderr)
		}
	}
}

func TestFgRefusesWithoutJobControl(t *testing.T) {
	result, output, stderr := shellFiles(t, t.TempDir(), `true & fg; echo "fg $?"`, nil)
	if result.Code != 0 || output != "fg 1\n" || !strings.Contains(stderr, "fg: no job control") {
		t.Fatalf("result %+v output %q stderr %q", result, output, stderr)
	}
}
