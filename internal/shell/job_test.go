package shell_test

import (
	"bytes"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/shell"
)

// A pipeline of in-process commands passes bytes on before its input ends.
func TestPipelineStreamsBeforeInputEnds(t *testing.T) {
	defer verifyNone(t)
	job, err := newShell().Start(shell.Spec{Script: "cat | cat", Dir: t.TempDir(), Env: testEnv(t)})
	if err != nil {
		t.Fatal(err)
	}
	input := []byte{0, 255, 128, 10}
	go job.Stdin().Write(input)
	got := make([]byte, len(input))
	read := make(chan error, 1)
	go func() {
		_, err := io.ReadFull(job.Stdout(), got)
		read <- err
	}()
	select {
	case err := <-read:
		if err != nil || !bytes.Equal(got, input) {
			t.Fatalf("read %v, %v", got, err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("the pipeline held its output until the end of input")
	}
	job.Stdin().Close()
	wantExit(t, collect(t, job), 0)
}

// Redirections, functions, subshells and cd make one shell session; the next
// job starts fresh.
func TestSessionStateIsPerJob(t *testing.T) {
	defer verifyNone(t)
	root := t.TempDir()
	sh := newShell()
	got := run(t, sh, root, `mkdir a b; f() { printf '%s\n' "$1"; }; f pear > a/input; (cd a; cat input) | tr a-z A-Z; cd b; export LEAK=bad`)
	wantExit(t, got, 0)
	if got.stdout != "PEAR\n" || got.result.Dir != filepath.Join(root, "b") {
		t.Fatalf("stdout %q, dir %q", got.stdout, got.result.Dir)
	}
	got = run(t, sh, got.result.Dir, `printf '%s' "${LEAK-unset}"; f 2>/dev/null || printf ' no f'`)
	if got.stdout != "unset no f" {
		t.Fatalf("stdout %q", got.stdout)
	}
}

// Utilities in a pipeline stream through it, and one also writes a file.
func TestUtilitiesStreamThroughPipelines(t *testing.T) {
	defer verifyNone(t)
	root := t.TempDir()
	got := run(t, newShell(), root, "printf hello | tee made.txt | cat | wc")
	wantExit(t, got, 0)
	if got.stdout != "5\n" || readFile(t, filepath.Join(root, "made.txt")) != "hello" {
		t.Fatalf("stdout %q", got.stdout)
	}
}

// A job reports completion only after its background work, and reports the
// foreground exit status.
func TestJobWaitsForBackgroundWork(t *testing.T) {
	defer verifyNone(t)
	root := t.TempDir()
	got := run(t, newShell(), root, "(sleep 0.05; echo completed > done) & exit 7")
	wantExit(t, got, 7)
	if readFile(t, filepath.Join(root, "done")) != "completed\n" {
		t.Fatal("background work did not finish before completion")
	}
}

// The caller sees output of the foreground before the background finishes,
// as in `(sleep 2; echo done) & echo started`.
func TestBackgroundOutputFollowsForegroundOutput(t *testing.T) {
	defer verifyNone(t)
	job, err := newShell().Start(shell.Spec{Script: "(sleep 0.2; echo done) & echo started", Dir: t.TempDir(), Env: testEnv(t)})
	if err != nil {
		t.Fatal(err)
	}
	job.Stdin().Close()
	line := make([]byte, len("started\n"))
	if _, err := io.ReadFull(job.Stdout(), line); err != nil || string(line) != "started\n" {
		t.Fatalf("first output %q, %v", line, err)
	}
	select {
	case <-job.Done():
		t.Fatal("the job completed before its background work")
	default:
	}
	got := collect(t, job)
	wantExit(t, got, 0)
	if got.stdout != "done\n" {
		t.Fatalf("rest of stdout %q", got.stdout)
	}
}

// Every job is a fresh login shell: the profiles apply, but the runner-owned
// variables, the aliases first in PATH and the working directory survive
// them.
func TestLoginProfilesDoNotReplaceTheJobContext(t *testing.T) {
	defer verifyNone(t)
	root := t.TempDir()
	if err := os.Mkdir(filepath.Join(root, "elsewhere"), 0o755); err != nil {
		t.Fatal(err)
	}
	aliases := filepath.Join(root, "aliases")
	system := filepath.Join(root, "system-profile")
	if err := os.WriteFile(system, []byte("export FROM_SYSTEM=yes\nsystem_function() { echo function; }\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	// A later user profile is not read when an earlier one is readable.
	if err := os.WriteFile(filepath.Join(root, ".profile"), []byte("export FROM_PROFILE=wrong\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	sh := shell.New(shell.Config{Utilities: fakeUtilities, SystemProfile: system, Umask: 0o022})
	for _, value := range []string{"first", "next"} {
		profile := "export FROM_PROFILE=" + value + "\nexport PATH=\"$HOME/tools\"\nexport DEMI_CONTEXT_ID=wrong TMPDIR=/wrong\ncd \"$HOME/elsewhere\"\n"
		if err := os.WriteFile(filepath.Join(root, ".bash_profile"), []byte(profile), 0o644); err != nil {
			t.Fatal(err)
		}
		got := runSpec(t, sh, shell.Spec{
			Script:   `printf '%s\n' "$FROM_SYSTEM" "$FROM_PROFILE" "$DEMI_CONTEXT_ID" "$TMPDIR" "$PATH"; system_function`,
			Dir:      root,
			Env:      []string{"HOME=" + root, "PATH=" + aliases + ":/usr/bin", "DEMI_CONTEXT_ID=owned", "TMPDIR=/owned"},
			Commands: &shell.Commands{Dispatcher: &fakeDispatcher{}, AliasDir: aliases},
		})
		wantExit(t, got, 0)
		want := strings.Join([]string{"yes", value, "owned", "/owned", aliases + ":" + filepath.Join(root, "tools"), "function", ""}, "\n")
		if got.stdout != want || got.result.Dir != root {
			t.Fatalf("stdout %q, want %q; dir %q", got.stdout, want, got.result.Dir)
		}
	}
}

// A profile that exits ends the job before the script runs.
func TestProfileExitEndsTheJob(t *testing.T) {
	defer verifyNone(t)
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, ".profile"), []byte("exit 4\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	got := runSpec(t, newShell(), shell.Spec{Script: "echo script", Dir: root, Env: []string{"HOME=" + root}})
	wantExit(t, got, 4)
	if got.stdout != "" {
		t.Fatalf("stdout %q", got.stdout)
	}
}

// Functions and compound pipelines drain large output and here-documents
// without truncation or deadlock.
func TestLargeOutputThroughPipelinesAndHereDocuments(t *testing.T) {
	defer verifyNone(t)
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "input"), bytes.Repeat([]byte("x"), 262144), 0o644); err != nil {
		t.Fatal(err)
	}
	script := "producer() { cat input; }; value=$(producer | cat | cat); printf '%s\\n' \"${#value}\"; { producer; } | wc; (producer) | wc; cat <<EOF | wc\n$value\nEOF\ncat <<< \"$value\" | wc"
	got := run(t, newShell(), root, script)
	wantExit(t, got, 0)
	if fields := strings.Fields(got.stdout); strings.Join(fields, " ") != "262144 262144 262144 262145 262145" {
		t.Fatalf("stdout %q", got.stdout)
	}
}

// Process substitution output reaches its consumer before the job completes.
func TestProcessSubstitutionOutputIsPreserved(t *testing.T) {
	defer verifyNone(t)
	root := t.TempDir()
	content := bytes.Repeat([]byte("x"), 256*1024)
	if err := os.WriteFile(filepath.Join(root, "input"), content, 0o644); err != nil {
		t.Fatal(err)
	}
	got := run(t, newShell(), root, "cat input | tee >(sleep 0.1; cat > copied) > /dev/null; cat <(echo in) <(echo process)")
	wantExit(t, got, 0)
	if got.stdout != "in\nprocess\n" || readFile(t, filepath.Join(root, "copied")) != string(content) {
		t.Fatalf("stdout %q, stderr %q", got.stdout, got.stderr)
	}
}

// A process substitution that nothing opens does not hold up the job, and an
// external program reads one through its /dev/fd path.
func TestProcessSubstitutionPaths(t *testing.T) {
	defer verifyNone(t)
	got := run(t, newShell(), t.TempDir(), "echo <(true) >/dev/null; /bin/cat <(echo external)")
	wantExit(t, got, 0)
	if got.stdout != "external\n" {
		t.Fatalf("stdout %q, stderr %q", got.stdout, got.stderr)
	}
}

// "/dev/stdout", "/dev/stderr" and "/dev/fd/N" name the job's descriptors,
// not the runner's.
func TestDescriptorPathsNameTheJobDescriptors(t *testing.T) {
	defer verifyNone(t)
	root := t.TempDir()
	got := run(t, newShell(), root, "echo to-stderr > /dev/stderr; exec 3>file; echo three > /dev/fd/3; cat /dev/fd/0 < file; tee /dev/stderr <<< tee")
	wantExit(t, got, 0)
	if got.stdout != "three\ntee\n" || got.stderr != "to-stderr\ntee\n" {
		t.Fatalf("stdout %q, stderr %q", got.stdout, got.stderr)
	}
}

// A syntax error reports status 2 without running anything.
func TestSyntaxError(t *testing.T) {
	defer verifyNone(t)
	got := run(t, newShell(), t.TempDir(), "echo before; if")
	wantExit(t, got, 2)
	if got.stdout != "" || got.stderr == "" {
		t.Fatalf("stdout %q, stderr %q", got.stdout, got.stderr)
	}
}

// Commands that do not exist or cannot run report Bash's statuses.
func TestMissingCommands(t *testing.T) {
	defer verifyNone(t)
	root := t.TempDir()
	got := run(t, newShell(), root, "no-such-command; echo $?; ./missing; echo $?; ./; echo $?")
	wantExit(t, got, 0)
	if got.stdout != "127\n127\n126\n" || !strings.Contains(got.stderr, "no-such-command: command not found") {
		t.Fatalf("stdout %q, stderr %q", got.stdout, got.stderr)
	}
}

// Starting a job in a directory that does not exist fails at once.
func TestStartRejectsAMissingDirectory(t *testing.T) {
	defer verifyNone(t)
	_, err := newShell().Start(shell.Spec{Script: "true", Dir: filepath.Join(t.TempDir(), "missing")})
	if err == nil {
		t.Fatal("Start succeeded")
	}
}
