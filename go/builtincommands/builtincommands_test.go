package builtincommands_test

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/go/builtincommands"
	"github.com/wspl/demi/go/builtinproto"
	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// hang is how long a test waits for an event before it decides that the code
// hangs. It guards against a hang; no test waits for it.
const hang = 30 * time.Second

// result is what an invocation left: its completion and its two outputs.
type result struct {
	completion commandservice.Completion
	stdout     string
	stderr     string
}

// message is what the agent reads of a failure.
func (r result) message() string {
	if r.completion.Error == nil {
		return ""
	}
	return r.completion.Error.Message
}

// job is a working directory whose file operations record their edits.
type job struct {
	t      *testing.T
	client *commandservice.Client
	dir    string
	edits  commandservice.EditContext
}

func newJob(t *testing.T) *job {
	t.Helper()
	server := servicetest.Start(t, builtincommands.New())
	dir := t.TempDir()
	return &job{
		t:      t,
		client: server.Client,
		dir:    dir,
		edits: commandservice.EditContext{
			Directory: filepath.Join(dir, "changes"),
			Lock:      filepath.Join(dir, "edits.lock"),
		},
	}
}

// call invokes an operation of the package with args, and returns what it left.
func (j *job) call(operation string, args any) result {
	j.t.Helper()
	document, err := json.Marshal(args)
	if err != nil {
		j.t.Fatal(err)
	}
	return j.callRaw(operation, document)
}

func (j *job) callRaw(operation string, document []byte) result {
	j.t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), hang)
	defer cancel()
	stream, err := j.client.Invoke(ctx, commandservice.Invocation{
		Operation:    operation,
		InvocationID: operation,
		Context: commandservice.CommandContext{
			Conversation: "file-test-conversation",
			Caller:       commandservice.AgentCaller{Number: 1},
			Locale:       commandservice.CommandLocale{TimeZone: "UTC", Languages: []string{"en-US"}},
		},
		Args:  jsontext.Value(document),
		Cwd:   j.dir,
		Env:   map[string]string{},
		Edits: &j.edits,
	})
	if err != nil {
		j.t.Fatalf("invoke %s: %v", operation, err)
	}
	var stdout, stderr bytes.Buffer
	completion, err := commandservice.Exchange(ctx, stream, bytes.NewReader(nil), &stdout, &stderr)
	if err != nil {
		j.t.Fatalf("%s: %v", operation, err)
	}
	return result{completion, stdout.String(), stderr.String()}
}

func (j *job) write(name, content string) {
	j.t.Helper()
	if err := os.WriteFile(filepath.Join(j.dir, name), []byte(content), 0o644); err != nil {
		j.t.Fatal(err)
	}
}

func (j *job) read(name string) string {
	j.t.Helper()
	data, err := os.ReadFile(filepath.Join(j.dir, name))
	if err != nil {
		j.t.Fatal(err)
	}
	return string(data)
}

// report is the journal the invocations left.
func (j *job) report() commandservice.EditJournal {
	j.t.Helper()
	recorder, err := commandservice.NewRecorder(j.edits)
	if err != nil {
		j.t.Fatal(err)
	}
	report, err := recorder.Report()
	if err != nil {
		j.t.Fatal(err)
	}
	return report
}

func (r result) succeeded(t *testing.T, want string) {
	t.Helper()
	if r.completion.ExitCode != 0 || r.completion.Error != nil || r.stdout != want {
		t.Errorf("exit code %d, error %+v, stdout %q, stderr %q; want success with %q", r.completion.ExitCode, r.completion.Error, r.stdout, r.stderr, want)
	}
}

func (r result) failed(t *testing.T, want string) {
	t.Helper()
	if r.completion.ExitCode != 1 || r.completion.Error == nil || r.completion.Error.Code != "command_failed" || r.message() != want {
		t.Errorf("exit code %d, error %+v; want command_failed %q", r.completion.ExitCode, r.completion.Error, want)
	}
}

func TestTheFileOperationsRunOnAWorkingDirectoryAndRecordWhatTheyEdit(t *testing.T) {
	j := newJob(t)
	j.call("file.create", map[string]any{"path": "nested/a.txt", "content": "alpha\nbeta\n"}).succeeded(t, "Created nested/a.txt\n")
	j.call("file.create", map[string]any{"path": "nested/a.txt", "content": "overwrite"}).failed(t, "File exists (os error 17)")
	j.call("file.edit", map[string]any{"path": "nested/a.txt", "old": "beta", "new": "gamma"}).succeeded(t, "Edited nested/a.txt\n")
	patch := "--- a/nested/a.txt\n+++ b/nested/a.txt\n@@ -1,2 +1,2 @@\n alpha\n-gamma\n+delta\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+created\n"
	j.call("file.patch", map[string]any{"patch": patch}).succeeded(t, "Patched 2 file(s)\n")
	j.call("file.read", map[string]any{"path": "nested/a.txt"}).succeeded(t, "alpha\ndelta\n")
	// The bytes of a file that is not text arrive as they are.
	binary := "\x00\xff\n\r\x80"
	j.write("image.bin", binary)
	j.call("file.read", map[string]any{"path": "image.bin"}).succeeded(t, binary)
	// A file over the record's limit is edited or refused, and never recorded.
	j.write("large.txt", strings.Repeat("x", commandservice.EditFileBytes+1))
	j.call("file.create", map[string]any{"path": "large.txt", "content": "overwrite"}).failed(t, "File exists (os error 17)")
	j.call("file.edit", map[string]any{"path": "large.txt", "old": "absent", "new": "replacement"}).failed(t, "No match found in large.txt")
	j.call("file.patch", map[string]any{"patch": "--- a/large.txt\n+++ b/large.txt\n@@ -1 +1 @@\n-absent\n+replacement\n"}).failed(t, "Patch does not apply to large.txt: Patch does not apply at line 1")

	report := j.report()
	if len(report.Files) != 2 {
		t.Fatalf("the journal lists %d files, want 2: %+v", len(report.Files), report)
	}
	first, second := report.Files[0], report.Files[1]
	if first.Kind != commandservice.EditAdded || len(first.Edits) != 1 {
		t.Fatalf("the first file = %+v, want one added file with one segment", first)
	}
	// The three writes of the first file are one segment of a file the job added:
	// nothing before it, and what the last write left after it.
	if data, err := os.ReadFile(*first.Edits[0].Modified); err != nil || string(data) != "alpha\ndelta\n" {
		t.Errorf("the first file's segment ends with %q, %v", data, err)
	}
	if data, err := os.ReadFile(*second.Edits[0].Modified); err != nil || string(data) != "created\n" {
		t.Errorf("the second file's segment ends with %q, %v", data, err)
	}
}

func TestAnEditReplacesTheOnlyOccurrenceTheOneAskedForOrTheOneNearestALine(t *testing.T) {
	j := newJob(t)
	j.write("file.txt", "one\ntwo\ntwo\n")
	j.call("file.edit", map[string]any{"path": "file.txt", "old": "two", "new": "changed"}).
		failed(t, "Multiple matches in file.txt; specify --occurrence or --context")
	j.call("file.edit", map[string]any{"path": "file.txt", "old": "two", "new": "changed", "occurrence": 2}).succeeded(t, "Edited file.txt\n")
	if got := j.read("file.txt"); got != "one\ntwo\nchanged\n" {
		t.Errorf("the second occurrence is replaced: %q", got)
	}
	j.call("file.edit", map[string]any{"path": "file.txt", "old": "two", "new": "x", "occurrence": 3}).failed(t, "Occurrence 3 is out of range")
	j.call("file.edit", map[string]any{"path": "file.txt", "old": "absent", "new": "x"}).failed(t, "No match found in file.txt")

	j.write("context.txt", "target\nmiddle\ntarget\n")
	j.call("file.edit", map[string]any{"path": "context.txt", "old": "target", "new": "changed", "context": 2}).failed(t,
		"Context line 2 is ambiguous: occurrence 1 at line 1; occurrence 2 at line 3")
	j.call("file.edit", map[string]any{"path": "context.txt", "old": "target", "new": "changed", "context": 3}).succeeded(t, "Edited context.txt\n")
	if got := j.read("context.txt"); got != "target\nmiddle\nchanged\n" {
		t.Errorf("the occurrence nearest to line 3 is replaced: %q", got)
	}
	j.call("file.edit", map[string]any{"path": "context.txt", "old": "absent", "new": "x", "context": 1}).failed(t, "No match found")

	// An edit that changes nothing writes nothing.
	j.write("same.txt", "same\n")
	before, err := os.Stat(filepath.Join(j.dir, "same.txt"))
	if err != nil {
		t.Fatal(err)
	}
	j.call("file.edit", map[string]any{"path": "same.txt", "old": "same", "new": "same"}).succeeded(t, "Edited same.txt\n")
	after, err := os.Stat(filepath.Join(j.dir, "same.txt"))
	if err != nil || !os.SameFile(before, after) {
		t.Errorf("an edit that changes nothing replaced the file: %v", err)
	}
}

func TestAnEditOfALinkWritesTheFileItPointsAtAndKeepsItsPermissions(t *testing.T) {
	j := newJob(t)
	j.write("target.txt", "one\n")
	if err := os.Chmod(filepath.Join(j.dir, "target.txt"), 0o640); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink("target.txt", filepath.Join(j.dir, "link.txt")); err != nil {
		t.Skipf("no symbolic links here: %v", err)
	}
	j.call("file.edit", map[string]any{"path": "link.txt", "old": "one", "new": "two"}).succeeded(t, "Edited link.txt\n")
	if got := j.read("target.txt"); got != "two\n" {
		t.Errorf("the target = %q", got)
	}
	if info, err := os.Lstat(filepath.Join(j.dir, "link.txt")); err != nil || info.Mode()&os.ModeSymlink == 0 {
		t.Errorf("the link was replaced by a file: %v", err)
	}
	if info, err := os.Stat(filepath.Join(j.dir, "target.txt")); err != nil || info.Mode().Perm() != 0o640 {
		t.Errorf("the target's permissions = %v, %v", info.Mode(), err)
	}
}

func TestACreateMakesTheDirectoriesAboveTheFileAndRefusesWhatItCannot(t *testing.T) {
	j := newJob(t)
	j.call("file.create", map[string]any{"path": "a/b/c.txt", "content": "text"}).succeeded(t, "Created a/b/c.txt\n")
	if got := j.read("a/b/c.txt"); got != "text" {
		t.Errorf("the file = %q", got)
	}
	// A file where a directory must be.
	j.call("file.create", map[string]any{"path": "a/b/c.txt/d.txt", "content": "text"}).failed(t, "File exists (os error 17)")
	j.call("file.create", map[string]any{"path": "a/b/c.txt/x/d.txt", "content": "text"}).failed(t, "Not a directory (os error 20)")
	// A path that is absolute names its own place, and one that goes up leaves
	// the working directory.
	outside := filepath.Join(t.TempDir(), "absolute.txt")
	j.call("file.create", map[string]any{"path": outside, "content": "nope"}).succeeded(t, "Created "+outside+"\n")
	if data, err := os.ReadFile(outside); err != nil || string(data) != "nope" {
		t.Errorf("the absolute file = %q, %v", data, err)
	}
	j.call("file.create", map[string]any{"path": "", "content": "nope"}).failed(t, "path must be nonempty and contain no NUL byte")
	j.call("file.read", map[string]any{"path": "a\x00b"}).failed(t, "path must be nonempty and contain no NUL byte")
}

func TestAReadThatCannotOpenOrReadTheFileSaysWhyInTheSystemsWords(t *testing.T) {
	j := newJob(t)
	j.call("file.read", map[string]any{"path": "missing.txt"}).failed(t, "No such file or directory (os error 2)")
	if err := os.Mkdir(filepath.Join(j.dir, "dir"), 0o755); err != nil {
		t.Fatal(err)
	}
	j.call("file.read", map[string]any{"path": "dir"}).failed(t, "Is a directory (os error 21)")
	j.call("file.edit", map[string]any{"path": "missing.txt", "old": "a", "new": "b"}).failed(t, "No such file or directory (os error 2)")
	j.write("binary.bin", "a\xffb")
	j.call("file.edit", map[string]any{"path": "binary.bin", "old": "a", "new": "b"}).failed(t, "stream did not contain valid UTF-8")
	// A read is sent on in pieces of the size a record holds.
	big := strings.Repeat("0123456789abcdef", 16*1024+1)
	j.write("big.txt", big)
	j.call("file.read", map[string]any{"path": "big.txt"}).succeeded(t, big)
}

func TestAPatchThatDoesNotFitLeavesEveryFileAsItWas(t *testing.T) {
	j := newJob(t)
	j.write("first.txt", "first\n")
	j.write("second.txt", "second\n")
	j.call("file.patch", map[string]any{"patch": "--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-first\n+changed\n" +
		"--- a/second.txt\n+++ b/second.txt\n@@ -1 +1 @@\n-wrong\n+changed\n"}).
		failed(t, "Patch does not apply to second.txt: Patch does not apply at line 1")
	if got := j.read("first.txt"); got != "first\n" {
		t.Errorf("first.txt = %q", got)
	}
	if report := j.report(); len(report.Files) != 0 {
		t.Errorf("a patch that changed nothing left %+v", report)
	}
}

func TestAPatchCreatesDeletesAndChangesFilesWhateverTheHeadersSay(t *testing.T) {
	// Each case starts from its own files, and lists the files it leaves; a file
	// it lists as "" is gone.
	for _, test := range []struct {
		name         string
		start, files map[string]string
		patch        string
		patched      int
	}{
		{
			"a header that ends with a time", map[string]string{"timed.txt": "old\n"}, map[string]string{"timed.txt": "new\n"},
			"--- a/timed.txt 2026-06-17 00:00:00.000000000 +0800\n+++ b/timed.txt 2026-06-17 00:00:01.000000000 +0800\n@@ -1 +1 @@\n-old\n+new\n",
			1,
		},
		{
			"a file that changes and one that is created", map[string]string{"existing.txt": "one\n"},
			map[string]string{"existing.txt": "changed\n", "nested/new.txt": "new\nfile\n"},
			"--- a/existing.txt\n+++ b/existing.txt\n@@ -1 +1 @@\n-one\n+changed\n--- /dev/null\n+++ b/nested/new.txt\n@@ -0,0 +1,2 @@\n+new\n+file\n",
			2,
		},
		{
			"a file that is deleted", map[string]string{"doomed.txt": "remove\n"}, map[string]string{"doomed.txt": ""},
			"--- a/doomed.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-remove\n",
			1,
		},
		{
			"a line without a newline at the end", map[string]string{"no-newline.txt": "a\nb"}, map[string]string{"no-newline.txt": "a\nc"},
			"--- a/no-newline.txt\n+++ b/no-newline.txt\n@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+c\n\\ No newline at end of file\n",
			1,
		},
		{
			"a file that moves", map[string]string{"moved.txt": "move\n"}, map[string]string{"renamed/moved.txt": "moved\n", "moved.txt": ""},
			"--- a/moved.txt\n+++ b/renamed/moved.txt\n@@ -1 +1 @@\n-move\n+moved\n",
			1,
		},
		{
			"git's lines between files, and a body line that looks like a header", map[string]string{"existing.txt": "changed\n"},
			map[string]string{"existing.txt": "changed\n++ not a header\n"},
			"diff --git a/existing.txt b/existing.txt\nindex 1..2 100644\n--- a/existing.txt\n+++ b/existing.txt\n@@ -1 +1,2 @@\n changed\n+++ not a header\n",
			1,
		},
		{
			"two hunks, the second after the lines the first added", map[string]string{"hunks.txt": "1\n2\n3\n4\n5\n"},
			map[string]string{"hunks.txt": "1\nx\ny\n2\n3\n4\nz\n"},
			"--- a/hunks.txt\n+++ b/hunks.txt\n@@ -1,2 +1,4 @@\n 1\n+x\n+y\n 2\n@@ -4,2 +6,2 @@\n 4\n-5\n+z\n",
			1,
		},
	} {
		j := newJob(t)
		for name, content := range test.start {
			if dir := filepath.Dir(name); dir != "." {
				if err := os.MkdirAll(filepath.Join(j.dir, dir), 0o755); err != nil {
					t.Fatal(err)
				}
			}
			j.write(name, content)
		}
		j.call("file.patch", map[string]any{"patch": test.patch}).succeeded(t, fmt.Sprintf("Patched %d file(s)\n", test.patched))
		for name, content := range test.files {
			data, err := os.ReadFile(filepath.Join(j.dir, name))
			if content == "" {
				if !os.IsNotExist(err) {
					t.Errorf("%s: %s remains: %v", test.name, name, err)
				}
				continue
			}
			if err != nil || string(data) != content {
				t.Errorf("%s: %s = %q, %v, want %q", test.name, name, data, err, content)
			}
		}
	}
}

func TestAPatchThatIsNotOneIsRefusedInTheWordsOfTheRust(t *testing.T) {
	j := newJob(t)
	j.write("a.txt", "one\ntwo\n")
	j.write("b.txt", "1\n2\n3\n4\n5\n")
	j.write("binary.bin", "a\xffb")
	j.write("other.txt", "other\n")
	const header = "--- a/a.txt\n+++ b/a.txt\n"
	for _, test := range []struct{ name, patch, want string }{
		{"nothing", "", "Invalid patch: missing file headers or hunks"},
		{"headers without hunks", header, "Invalid patch: missing file headers or hunks"},
		{"an old header alone", "--- a/a.txt\n", "Invalid patch: missing file headers or hunks"},
		{"a new header alone", "+++ b/a.txt\n", "New header precedes old header"},
		{"a hunk before headers", "@@ -1 +1 @@\n", "Hunk before file header"},
		{"a header that is not one", header + "@@ nonsense\n", "Invalid patch hunk header"},
		{"a start that does not fit", header + "@@ -99999999999999999999 +1 @@\n", "Invalid hunk start"},
		{"a count that does not fit", header + "@@ -1,99999999999999999999 +1 @@\n", "Invalid hunk count"},
		{"a line that is none", header + "@@ -1 +1 @@\nnonsense\n", "Invalid patch line: nonsense"},
		{"a marker without a line", header + "@@ -1 +1 @@\n\\ No newline at end of file\n", "Newline marker without patch line"},
		{"counts that do not add up", header + "@@ -1,2 +1 @@\n-one\n", "Patch does not apply to a.txt: Patch hunk line counts do not match header"},
		{"lines that are not there", header + "@@ -1 +1 @@\n-wrong\n+x\n", "Patch does not apply to a.txt: Patch does not apply at line 1"},
		{"a hunk past the end", header + "@@ -9 +9 @@\n-wrong\n+x\n", "Patch does not apply to a.txt: Patch does not apply at line 9"},
		{"a hunk before the start of the file",
			"--- a/b.txt\n+++ b/b.txt\n@@ -3,3 +3,0 @@\n-3\n-4\n-5\n@@ -1 +1 @@\n-1\n+x\n",
			"Patch does not apply to b.txt: Patch hunk starts before file"},
		{"a delete that leaves content", "--- a/a.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-one\n", "Delete patch leaves file content"},
		{"a destination that exists", "--- a/a.txt\n+++ b/other.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+2\n", "Patch destination already exists"},
		{"a path changed twice",
			"--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+1\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+uno\n",
			"Patch changes the same path more than once"},
		{"a file that is not text", "--- a/binary.bin\n+++ b/binary.bin\n@@ -1 +1 @@\n-a\n+b\n", "invalid utf-8 sequence of 1 bytes from index 1"},
		{"a file that is missing", "--- a/missing.txt\n+++ b/missing.txt\n@@ -1 +1 @@\n-a\n+b\n", "No such file or directory (os error 2)"},
		{"a path that is empty", "--- a/\n+++ b/\n@@ -1 +1 @@\n-a\n+b\n", "path must be nonempty and contain no NUL byte"},
	} {
		result := j.call("file.patch", map[string]any{"patch": test.patch})
		if result.completion.ExitCode != 1 || result.message() != test.want {
			t.Errorf("%s: exit code %d, %q; want %q", test.name, result.completion.ExitCode, result.message(), test.want)
		}
	}
	if got := j.read("a.txt"); got != "one\ntwo\n" {
		t.Errorf("a refused patch changed a.txt: %q", got)
	}
}

func TestAnArgumentThatBreaksItsTypeIsAFailureNamingTheField(t *testing.T) {
	j := newJob(t)
	j.callRaw("file.read", []byte(`{}`)).failed(t, "invalid: path: required")
	j.callRaw("file.edit", []byte(`{"path":"a","old":"","new":"b"}`)).failed(t, "invalid: old: must have at least 1 character")
	j.callRaw("file.edit", []byte(`{"path":"a","old":"x","new":"b","occurrence":0}`)).failed(t, "invalid: occurrence: must be at least 1")
	j.callRaw("file.patch", []byte(`{"patch":1}`)).failed(t, "invalid: patch: must be a string")
}

func TestTheProgramListsEveryOperationOfThePackage(t *testing.T) {
	handler := builtincommands.New()
	names := handler.Operations()
	if len(names) != 52 || names[0] != "file.read" || names[3] != "file.patch" || names[len(names)-1] != "browser.live" {
		t.Errorf("the operations = %v", names)
	}
	j := newJob(t)
	if result := j.call("browser.tabs", map[string]any{}); result.completion.ExitCode != 1 || !strings.Contains(result.message(), "browser.tabs is served by the conversation browser") {
		t.Errorf("a browser operation answers %+v", result.completion)
	}
}

func TestFileMutationsRunOneAtATimeSoNoEditIsLost(t *testing.T) {
	j := newJob(t)
	const edits = 24
	var content strings.Builder
	for i := range edits {
		fmt.Fprintf(&content, "line-%d\n", i)
	}
	j.write("shared.txt", content.String())
	var wg sync.WaitGroup
	for i := range edits {
		wg.Add(1)
		go func() {
			defer wg.Done()
			ctx, cancel := context.WithTimeout(context.Background(), hang)
			defer cancel()
			document, _ := json.Marshal(map[string]any{"path": "shared.txt", "old": fmt.Sprintf("line-%d\n", i), "new": fmt.Sprintf("edited-%d\n", i)})
			stream, err := j.client.Invoke(ctx, commandservice.Invocation{
				Operation: "file.edit", InvocationID: fmt.Sprint(i),
				Context: commandservice.CommandContext{Conversation: "c", Caller: commandservice.UserCaller{}, Locale: commandservice.CommandLocale{TimeZone: "UTC", Languages: []string{"en"}}},
				Args:    jsontext.Value(document), Cwd: j.dir, Env: map[string]string{},
			})
			if err != nil {
				t.Errorf("invoke: %v", err)
				return
			}
			completion, err := commandservice.Exchange(ctx, stream, bytes.NewReader(nil), &bytes.Buffer{}, &bytes.Buffer{})
			if err != nil || completion.ExitCode != 0 {
				t.Errorf("edit %d: %+v, %v", i, completion, err)
			}
		}()
	}
	wg.Wait()
	for i := range edits {
		if !strings.Contains(j.read("shared.txt"), fmt.Sprintf("edited-%d\n", i)) {
			t.Errorf("the edit %d is lost: %q", i, j.read("shared.txt"))
			return
		}
	}
}

func TestAnInvocationThatAsksForNoRecordingRecordsNothing(t *testing.T) {
	server := servicetest.Start(t, builtincommands.New())
	dir := t.TempDir()
	ctx, cancel := context.WithTimeout(context.Background(), hang)
	defer cancel()
	stream, err := server.Client.Invoke(ctx, commandservice.Invocation{
		Operation: "file.create", InvocationID: "create",
		Context: commandservice.CommandContext{Conversation: "c", Caller: commandservice.UserCaller{}, Locale: commandservice.CommandLocale{TimeZone: "UTC", Languages: []string{"en"}}},
		Args:    jsontext.Value(`{"path":"a.txt","content":"a"}`), Cwd: dir, Env: map[string]string{},
	})
	if err != nil {
		t.Fatal(err)
	}
	var stdout bytes.Buffer
	completion, err := commandservice.Exchange(ctx, stream, bytes.NewReader(nil), &stdout, &bytes.Buffer{})
	if err != nil || completion.ExitCode != 0 || stdout.String() != "Created a.txt\n" {
		t.Fatalf("%+v, %v, %q", completion, err, stdout.String())
	}
	if _, err := os.Stat(filepath.Join(dir, "changes")); !os.IsNotExist(err) {
		t.Errorf("a recording was made: %v", err)
	}
	_ = builtinproto.Package
}
