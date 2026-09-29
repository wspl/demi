package shell_test

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/go/runner/shell"
	"golang.org/x/sys/windows"
	"mvdan.cc/sh/v3/syntax"
)

// The parent and grandchild both block on the job's still-open stdin. The test
// observes the grandchild's process handle before cancellation, then waits for
// its exit. This requires a native Windows worker; cross-compilation is not proof.
func TestWindowsDescendantContainment(t *testing.T) {
	opts := options(t)
	input, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer writer.Close()
	opts.Stdin = input
	sink := eventSink{make(chan []byte, 4)}
	opts.Output = sink
	exe, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	quoted, err := syntax.Quote(exe, syntax.LangBash)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	result := make(chan shell.Result, 1)
	go func() { result <- shell.Run(ctx, quoted+" --shell-test-parent", opts) }()
	var line string
	for !strings.Contains(line, "\n") {
		select {
		case bytes := <-sink.received:
			line += string(bytes)
		case <-time.After(5 * time.Second):
			t.Fatal("grandchild did not start")
		}
	}
	pid, err := strconv.Atoi(strings.TrimSpace(line))
	if err != nil {
		t.Fatal(line)
	}
	process, err := windows.OpenProcess(windows.SYNCHRONIZE, false, uint32(pid))
	if err != nil {
		t.Fatal(err)
	}
	defer windows.CloseHandle(process)
	cancel()
	select {
	case got := <-result:
		if got.Signal != "SIGKILL" {
			t.Fatal(got)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("job did not cancel")
	}
	status, err := windows.WaitForSingleObject(process, 5000)
	if err != nil || status != windows.WAIT_OBJECT_0 {
		t.Fatalf("descendant survived: %d %v", status, err)
	}
}

func windowsFixture() {
	if len(os.Args) != 2 {
		return
	}
	switch os.Args[1] {
	case "--shell-test-parent":
		cmd := exec.Command(os.Args[0], "--shell-test-grandchild")
		cmd.Stdin, cmd.Stdout, cmd.Stderr = os.Stdin, os.Stdout, os.Stderr
		if err := cmd.Run(); err != nil {
			os.Exit(1)
		}
		os.Exit(0)
	case "--shell-test-grandchild":
		fmt.Println(os.Getpid())
		var buffer [1]byte
		os.Stdin.Read(buffer[:])
		os.Exit(0)
	}
}

func TestWindowsDrivePaths(t *testing.T) {
	opts := options(t)
	directory := filepath.Join(opts.Dir, "with spaces")
	if err := os.Mkdir(directory, 0700); err != nil {
		t.Fatal(err)
	}
	quoted, err := syntax.Quote(directory, syntax.LangBash)
	if err != nil {
		t.Fatal(err)
	}
	result, out := execute(t, opts, "cd "+quoted+"; echo drive > note; cmd.exe /c type note")
	if result.Code == nil || *result.Code != 0 || strings.TrimSpace(out.stdout.String()) != "drive" {
		t.Fatalf("%+v %q %q", result, out.stdout.String(), out.stderr.String())
	}
}

// The Rust's windows_drive_paths_work_for_cd_redirection_utilities_and_executables:
// /dev/null and drive paths (/c/...) work for redirections, cd and programs.
// The Rust's cat is a builtin here only through read.
// Cost: one job and one cmd.exe run.
func TestWindowsDrivePathsForRedirectionsCdAndPrograms(t *testing.T) {
	drive := func(path string) string {
		native := filepath.ToSlash(path)
		if native[1:3] != ":/" {
			t.Fatalf("not a drive path: %s", path)
		}
		return "/" + native[:1] + "/" + native[3:]
	}
	opts := options(t)
	if err := os.Mkdir(filepath.Join(opts.Dir, "sub"), 0700); err != nil {
		t.Fatal(err)
	}
	opts.Env["DRIVE_ROOT"] = drive(opts.Dir)
	opts.Env["DRIVE_EXE"] = drive(filepath.Join(os.Getenv("SystemRoot"), "System32", "cmd.exe"))
	result, out := execute(t, opts, `printf discarded > /dev/null && printf discarded &> /dev/null && printf payload > "$DRIVE_ROOT/file" && cd "$DRIVE_ROOT/sub" && { IFS= read -r line < "$DRIVE_ROOT/file" || true; } && printf '%s' "$line" && "$DRIVE_EXE" /c "echo external"`)
	if *result.Code != 0 || result.Dir != filepath.Join(opts.Dir, "sub") || out.stdout.String() != "payloadexternal\r\n" {
		t.Fatalf("%+v %q %q", result, out.stdout.String(), out.stderr.String())
	}
}
