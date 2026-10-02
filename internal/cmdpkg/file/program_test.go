package file

import (
	"bytes"
	"errors"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/cmdpkg/file/fileop"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/cmdsdk/cmdsdktest"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/programtest"
	"go.uber.org/goleak"
)

type programSuite struct{ m *testing.M }

func (s programSuite) Run() int { return programtest.Run(s.m) }
func TestMain(m *testing.M)     { goleak.VerifyTestMain(programSuite{m}) }

type argument interface{ MarshalJSON() ([]byte, error) }

// callFile leaves stdin open to verify that file operations never request it.
func callFile(t *testing.T, client *cmdsdk.Client, cwd, operation string, args argument) (commandwire.Completion, []byte) {
	t.Helper()
	body, err := args.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	input, output, err := client.Invoke(t.Context(), commandwire.Invocation{
		Context: commandwire.CommandContext{Conversation: "file-test-conversation", Caller: &commandwire.AgentCaller{Number: 1}, Locale: commandwire.CommandLocale{TimeZone: "UTC", Languages: []commandwire.LanguageTag{"en-US"}}},
		Edits:   editContext(cwd), Operation: operation, InvocationID: operation, Args: body, Cwd: cwd, Env: map[string]string{},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer input.Cancel()
	return collect(t, output)
}

// collect observes the file program's output and completion at the wire boundary.
func collect(t *testing.T, output *cmdsdk.CommandOutput) (commandwire.Completion, []byte) {
	t.Helper()
	var stdout []byte
	var completion commandwire.Completion
	for {
		record, err := output.Next(t.Context())
		if errors.Is(err, io.EOF) {
			return completion, stdout
		}
		if err != nil {
			t.Fatal(err)
		}
		switch r := record.(type) {
		case commandwire.Stdout:
			stdout = append(stdout, r...)
		case commandwire.Stderr:
			t.Fatalf("unexpected stderr: %s", r)
		case commandwire.Completed:
			completion = r.Completion
		case commandwire.InputPull:
			t.Fatal("file operation requested raw input")
		}
	}
}
func editContext(cwd string) *commandwire.EditContext {
	return &commandwire.EditContext{Directory: filepath.Join(cwd, "changes"), Lock: filepath.Join(cwd, "edits.lock")}
}
func startFile(t *testing.T) *cmdsdktest.ServiceProcess {
	t.Helper()
	path, err := programtest.Path(t.Context(), "demi-file")
	if err != nil {
		t.Fatal(err)
	}
	process, err := cmdsdktest.Start(t.Context(), t, path, []string{"--command-service"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	return process
}
func writeFixture(t *testing.T, path, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0600); err != nil {
		t.Fatal(err)
	}
}
func contents(t *testing.T, path string) []byte {
	t.Helper()
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return b
}
func report(t *testing.T, cwd string) commandwire.EditJournal {
	t.Helper()
	recorder, err := cmdsdk.NewRecorder(t.Context(), *editContext(cwd))
	if err != nil {
		t.Fatal(err)
	}
	journal, err := recorder.Report(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	return journal
}

// The real resident executable proves operation dispatch, recording and shutdown.
// Its single shared incremental program build dominates this scenario's cost.
func TestResidentProgramServesEveryFileOperationAndRecordsEdits(t *testing.T) {
	cwd := t.TempDir()
	process := startFile(t)
	pid := process.PID()
	info, err := process.Client.Info(t.Context())
	if err != nil || !slices.Equal(info.Operations, fileop.Operations()) {
		t.Fatalf("info=%+v error=%v", info, err)
	}
	result, output := callFile(t, process.Client, cwd, "file.create", fileop.CreateArgs{Path: "nested/a.txt", Content: "alpha\nbeta\n"})
	if result.ExitCode != 0 || string(output) != "Created nested/a.txt\n" {
		t.Fatalf("%+v %q", result, output)
	}
	result, _ = callFile(t, process.Client, cwd, "file.create", fileop.CreateArgs{Path: "nested/a.txt", Content: "overwrite"})
	if result.ExitCode != 1 {
		t.Fatal(result)
	}
	result, _ = callFile(t, process.Client, cwd, "file.edit", fileop.EditArgs{Path: "nested/a.txt", Old: "beta", New: "gamma"})
	if result.ExitCode != 0 {
		t.Fatal(result)
	}
	patch := "--- a/nested/a.txt\n+++ b/nested/a.txt\n@@ -1,2 +1,2 @@\n alpha\n-gamma\n+delta\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+created\n"
	result, output = callFile(t, process.Client, cwd, "file.patch", fileop.PatchArgs{Patch: patch})
	if result.ExitCode != 0 || string(output) != "Patched 2 file(s)\n" {
		t.Fatalf("%+v %q", result, output)
	}
	result, output = callFile(t, process.Client, cwd, "file.read", fileop.ReadArgs{Path: "nested/a.txt"})
	if result.ExitCode != 0 || string(output) != "alpha\ndelta\n" {
		t.Fatalf("%+v %q", result, output)
	}
	binary := []byte{0, 255, 10, 13, 128}
	writeFixture(t, filepath.Join(cwd, "image.bin"), string(binary))
	result, output = callFile(t, process.Client, cwd, "file.read", fileop.ReadArgs{Path: "image.bin"})
	if result.ExitCode != 0 || !bytes.Equal(output, binary) {
		t.Fatalf("%+v %q", result, output)
	}
	journal := report(t, cwd)
	if len(journal.Files) != 2 || journal.Files[0].Kind != commandwire.EditAdded || len(journal.Files[0].Edits) != 1 {
		t.Fatalf("%+v", journal)
	}
	for i, want := range []string{"alpha\ndelta\n", "created\n"} {
		copyPath := journal.Files[i].Edits[0].Modified
		if copyPath == nil || string(contents(t, *copyPath)) != want {
			t.Fatalf("snapshot %d: %+v", i, journal.Files[i])
		}
	}
	writeFixture(t, filepath.Join(cwd, "large.txt"), strings.Repeat("x", commandwire.EditFileBytes+1))
	for _, c := range []struct {
		operation string
		args      argument
	}{
		{"file.create", fileop.CreateArgs{Path: "large.txt", Content: "overwrite"}},
		{"file.edit", fileop.EditArgs{Path: "large.txt", Old: "absent", New: "replacement"}},
		{"file.patch", fileop.PatchArgs{Patch: "--- a/large.txt\n+++ b/large.txt\n@@ -1 +1 @@\n-absent\n+replacement\n"}},
	} {
		result, _ = callFile(t, process.Client, cwd, c.operation, c.args)
		if result.ExitCode != 1 {
			t.Fatal(result)
		}
	}
	if got := report(t, cwd); len(got.Files) != 2 {
		t.Fatalf("failed operations recorded edits: %+v", got)
	}
	_, status, err := process.Client.Conversation(t.Context(), &commandwire.ConversationQuery{})
	if err != nil {
		t.Fatal(err)
	}
	result, output = collect(t, status)
	if result.ExitCode != 0 || string(output) != `{"conversations":[]}` {
		t.Fatalf("%+v %s", result, output)
	}
	if process.PID() != pid {
		t.Fatal("resident process changed")
	}
	if err := process.Shutdown(t.Context()); err != nil {
		t.Fatal(err)
	}
}

// This uses a real filesystem publication failure after an earlier successful write.
func TestPatchFailureRestoresEarlierFiles(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("Rust scenario is Unix-only")
	}
	cwd := t.TempDir()
	writeFixture(t, filepath.Join(cwd, "first.txt"), "first\n")
	locked := filepath.Join(cwd, "locked")
	name, text := "second.txt", "second\n"
	if runtime.GOOS == "linux" {
		if err := os.Symlink("/proc/sys/kernel", locked); err != nil {
			t.Fatal(err)
		}
		name = "ostype"
		text = string(contents(t, filepath.Join(locked, name)))
	} else {
		if err := os.Mkdir(locked, 0755); err != nil {
			t.Fatal(err)
		}
		writeFixture(t, filepath.Join(locked, name), text)
		if err := os.Chmod(locked, 0555); err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() {
			if err := os.Chmod(locked, 0755); err != nil {
				t.Error(err)
			}
		})
	}
	process := startFile(t)
	patch := "--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-first\n+changed\n--- a/locked/" + name + "\n+++ b/locked/" + name + "\n@@ -1 +1 @@\n-" + strings.TrimSpace(text) + "\n+changed\n"
	result, _ := callFile(t, process.Client, cwd, "file.patch", fileop.PatchArgs{Patch: patch})
	if result.ExitCode != 1 {
		t.Fatal(result)
	}
	if string(contents(t, filepath.Join(cwd, "first.txt"))) != "first\n" || string(contents(t, filepath.Join(locked, name))) != text {
		t.Fatal("patch did not roll back")
	}
	if journal := report(t, cwd); len(journal.Files) != 0 {
		t.Fatalf("rollback recorded: %+v", journal)
	}
	if err := process.Shutdown(t.Context()); err != nil {
		t.Fatal(err)
	}
}
