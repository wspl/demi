//go:build difftest

package builtincommands_test

// The differential judge of demi-commands: the Go program and the Rust program
// are started with --command-service, driven through the Go command-service
// client with the same invocations in working directories that start alike, and
// what they answer and what they leave are compared. It runs when the Rust
// program is named:
//
//	DEMI_DIFFTEST_RUST=<demi-commands> go test -tags difftest -run Differential ./builtincommands
//
// DEMI_DIFFTEST_GO names the Go program; without it the test builds one.
//
// Volatile values are normalized by one rule: the paths of the run (the root
// directory of a program's run, and its working directory) are replaced by {root}
// and {dir}, in what a program answers, in the files it leaves and in the
// journal of its edits. Nothing else is: bytes, exit codes, error codes, error
// messages and the files' contents and permissions are compared as they are.
//
// The table grows with each part of the Go migration: G4a is the file
// operations; the catalog of browser commands and the live view's commands
// follow with G4b and G4c.

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"testing"

	"github.com/wspl/demi/go/builtinproto"
	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// A step is one invocation of the table: an operation and its arguments, in
// which {dir} and {root} stand for the run's directories.
type step struct {
	operation, args string
	// noEdits leaves out the edit context, as an invocation of a job that
	// records no edits does.
	noEdits bool
	// messageDiffers says why the error message of a refused argument is not
	// compared: the Go program words it as the wire contract does (which names
	// the field and the rule), where the Rust's words are serde's and garde's.
	// The exit code and the error code are still compared.
	messageDiffers string
}

// A scenario is a working directory that starts with files, and the invocations
// run in it in order.
type scenario struct {
	name string
	// files are the files the directory starts with, by path in the directory;
	// a name that ends in / is an empty directory, and a value that starts with
	// "->" is a symbolic link to the rest.
	files map[string]string
	// modes are the permissions of files the directory starts with, when they
	// are not the default.
	modes map[string]fs.FileMode
	steps []step
}

// An observation is what a program answered to one invocation.
type observation struct {
	exitCode  int
	errorCode string
	message   string
	stdout    string
	stderr    string
}

// A run is what a program did with a scenario.
type run struct {
	steps   []observation
	tree    map[string]string
	journal []string
}

// quote returns text as a JSON string.
func quote(text string) string {
	quoted, err := json.Marshal(text)
	if err != nil {
		panic(err)
	}
	return string(quoted)
}

func read(path string) step {
	return step{operation: "file.read", args: `{"path":` + quote(path) + `}`}
}

func create(path, content string) step {
	return step{operation: "file.create", args: `{"path":` + quote(path) + `,"content":` + quote(content) + `}`}
}

func edit(path, old, replacement string, extra string) step {
	return step{operation: "file.edit", args: `{"path":` + quote(path) + `,"old":` + quote(old) + `,"new":` + quote(replacement) + extra + `}`}
}

func patch(text string) step {
	return step{operation: "file.patch", args: `{"patch":` + quote(text) + `}`}
}

// table is the invocation table of the file operations. Each scenario is a
// behavior the Rust tests pin, or a refusal the Rust words.
var table = []scenario{
	{
		// resident_executable_runs_all_builtin_file_operations
		name: "the resident program's file operations",
		steps: []step{
			create("nested/a.txt", "alpha\nbeta\n"),
			create("nested/a.txt", "overwrite"),
			edit("nested/a.txt", "beta", "gamma", ""),
			patch("--- a/nested/a.txt\n+++ b/nested/a.txt\n@@ -1,2 +1,2 @@\n alpha\n-gamma\n+delta\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+created\n"),
			read("nested/a.txt"),
			read("image.bin"),
			create("large.txt", "overwrite"),
			edit("large.txt", "absent", "replacement", ""),
			patch("--- a/large.txt\n+++ b/large.txt\n@@ -1 +1 @@\n-absent\n+replacement\n"),
		},
		files: map[string]string{"image.bin": "\x00\xff\n\r\x80", "large.txt": strings.Repeat("x", commandservice.EditFileBytes+1)},
	},
	{
		// demi_file_reads_and_creates_files_in_and_beyond_the_workspace
		name: "create and read, in and beyond the working directory",
		files: map[string]string{
			"shot.png": "\x89PNG\r\n\x1a\n\x00\xff\xfe",
		},
		steps: []step{
			create("note.txt", "hello world\n"),
			read("note.txt"),
			create("note.txt", "again\n"),
			create("src/foo.txt", "hello\n"),
			read("src/foo.txt"),
			create("{dir}/absolute.txt", "nope\n"),
			create("../relative.txt", "nope\n"),
			read("shot.png"),
			create("empty.txt", ""),
			read("empty.txt"),
			create("unicode/héllo €.txt", "déjà vu \U0001F600\n"),
			read("unicode/héllo €.txt"),
			create("./dotted/../plain.txt", "dots\n"),
			create("taken/x.txt", "x"),
			create("taken/x.txt/deeper.txt", "x"),
			create("taken/x.txt/deeper/again.txt", "x"),
		},
	},
	{
		name: "reads that fail",
		files: map[string]string{
			"dir/":       "",
			"secret.txt": "text",
		},
		steps: []step{
			read("missing.txt"),
			read("dir"),
			read(""),
			read("a\x00b"),
			read("dir/../secret.txt"),
			read("nope/../secret.txt"),
			create("", "x"),
		},
	},
	{
		name: "a read of a file of many records",
		files: map[string]string{
			"big.txt":   strings.Repeat("0123456789abcdef", 16*1024*3+7),
			"exact.txt": strings.Repeat("y", 64*1024),
		},
		steps: []step{read("big.txt"), read("exact.txt")},
	},
	{
		// demi_file_edit_and_patch_change_what_they_name_whole_or_not_at_all
		name: "edits replace one exact match",
		files: map[string]string{
			"file.txt":      "one\ntwo\ntwo\n",
			"context.txt":   "target\nmiddle\ntarget\n",
			"empty-old.txt": "content\n",
			"tail.txt":      "no newline at the end",
			"aaa.txt":       "aaaa\n",
			"unicode.txt":   "éé x\n€ x\n\U0001F600 x\n",
			"dir/":          "",
			"bin.txt":       "a\xffb",
			"link.txt":      "->file.txt",
			"lines.txt":     "x\nx\nx\nx\nx\n",
		},
		steps: []step{
			edit("file.txt", "two", "changed", ""),
			edit("file.txt", "two", "changed", `,"occurrence":2`),
			read("file.txt"),
			edit("file.txt", "two", "x", `,"occurrence":3`),
			edit("file.txt", "absent", "x", ""),
			edit("file.txt", "absent", "x", `,"occurrence":1`),
			edit("context.txt", "target", "changed", `,"context":2`),
			edit("context.txt", "target", "changed", `,"context":3`),
			read("context.txt"),
			edit("context.txt", "absent", "x", `,"context":1`),
			edit("context.txt", "middle", "MIDDLE", `,"context":100`),
			edit("lines.txt", "x", "y", `,"context":3`),
			edit("lines.txt", "x", "y", `,"context":4`),
			edit("lines.txt", "x", "y", `,"context":1,"occurrence":5`),
			edit("lines.txt", "x", "y", `,"occurrence":18446744073709551615`),
			edit("lines.txt", "x", "y", `,"context":18446744073709551615`),
			read("lines.txt"),
			edit("empty-old.txt", "content", "content", ""),
			edit("tail.txt", "end", "END", ""),
			edit("tail.txt", "no newline", "a newline\nand more", ""),
			read("tail.txt"),
			edit("aaa.txt", "aa", "b", ""),
			edit("aaa.txt", "aa", "b", `,"occurrence":2`),
			read("aaa.txt"),
			edit("unicode.txt", "x", "y", `,"context":2`),
			edit("unicode.txt", "€ x", "euro", ""),
			read("unicode.txt"),
			edit("bin.txt", "a", "b", ""),
			edit("dir", "a", "b", ""),
			edit("missing.txt", "a", "b", ""),
			edit("link.txt", "one", "uno", ""),
			read("file.txt"),
			edit("file.txt", "uno", "", ""),
			edit("file.txt", "\n", "", `,"occurrence":1`),
		},
	},
	{
		name: "an edit that changes nothing, and jobs that record no edits",
		files: map[string]string{
			"same.txt": "same\n",
			"a.txt":    "a\n",
		},
		steps: []step{
			edit("same.txt", "same", "same", ""),
			{operation: "file.edit", args: `{"path":"a.txt","old":"a","new":"b"}`, noEdits: true},
			{operation: "file.create", args: `{"path":"b.txt","content":"b"}`, noEdits: true},
			{operation: "file.patch", args: `{"patch":"--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-b\n+c\n"}`, noEdits: true},
		},
	},
	{
		name: "patches that apply",
		files: map[string]string{
			"patch.txt":      "one\ntwo\n",
			"timed.txt":      "old\n",
			"existing.txt":   "one\n",
			"doomed.txt":     "remove\n",
			"nonl.txt":       "a\nb",
			"moved.txt":      "move\n",
			"hunks.txt":      "1\n2\n3\n4\n5\n6\n7\n8\n",
			"crlf.txt":       "a\r\nb\r\n",
			"empty.txt":      "",
			"space name.txt": "s\n",
			"exec.sh":        "echo\n",
			"tabbed.txt":     "t\n",
			"unicode.txt":    "héllo\n€\n",
			"git.txt":        "g\n",
		},
		steps: []step{
			patch("--- a/patch.txt\n+++ b/patch.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+three\n"),
			patch("--- a/timed.txt 2026-06-17 00:00:00.000000000 +0800\n+++ b/timed.txt 2026-06-17 00:00:01.000000000 +0800\n@@ -1 +1 @@\n-old\n+new\n"),
			patch("--- a/existing.txt\n+++ b/existing.txt\n@@ -1 +1 @@\n-one\n+changed\n--- /dev/null\n+++ b/nested/new.txt\n@@ -0,0 +1,2 @@\n+new\n+file\n"),
			patch("--- a/doomed.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-remove\n"),
			patch("--- a/nonl.txt\n+++ b/nonl.txt\n@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+c\n\\ No newline at end of file\n"),
			patch("--- a/moved.txt\n+++ b/renamed/moved.txt\n@@ -1 +1 @@\n-move\n+moved\n"),
			patch("--- a/hunks.txt\n+++ b/hunks.txt\n@@ -1,2 +1,4 @@\n 1\n+x\n+y\n 2\n@@ -3 +5 @@\n-3\n+three\n@@ -6,3 +8,2 @@\n 6\n-7\n 8\n"),
			patch("--- a/crlf.txt\n+++ b/crlf.txt\n@@ -1,2 +1,2 @@\n a\r\n-b\r\n+c\r\n"),
			patch("--- a/empty.txt\n+++ b/empty.txt\n@@ -0,0 +1 @@\n+filled\n"),
			patch("--- a/space name.txt\n+++ b/space name.txt\n@@ -1 +1 @@\n-s\n+spaced\n"),
			patch("--- exec.sh\t2026-01-01 00:00:00\n+++ exec.sh\t2026-01-01 00:00:01\n@@ -1 +1 @@\n-echo\n+echo done\n"),
			patch("--- a/tabbed.txt\tMon Jan 1 2026\n+++ b/tabbed.txt\tMon Jan 1 2026\n@@ -1 +1 @@\n-t\n+tabbed\n"),
			patch("--- a/unicode.txt\n+++ b/unicode.txt\n@@ -1,2 +1,2 @@\n héllo\n-€\n+euro\n"),
			patch("diff --git a/git.txt b/git.txt\nindex 1..2 100644\n--- a/git.txt\n+++ b/git.txt\n@@ -1 +1,2 @@\n g\n+++ not a header\n"),
			patch("--- /dev/null\n+++ b/created/deep/er.txt\n@@ -0,0 +1 @@\n+new\n"),
			patch("--- /dev/null\n+++ b/empty-new.txt\n@@ -0,0 +0,0 @@\n"),
			patch("--- a/patch.txt\n+++ b/patch.txt\n@@ -1,2 +1,2 @@\n one\n-three\n+two\n\n\n"),
			patch("--- {dir}/patch.txt\n+++ {dir}/patch.txt\n@@ -2 +2 @@\n-two\n+2\n"),
			patch("\n\n--- a/patch.txt\n+++ b/patch.txt\n\n@@ -1 +1 @@\n-one\n+1\n\n"),
			read("patch.txt"),
			read("hunks.txt"),
		},
	},
	{
		name: "patches that are refused",
		files: map[string]string{
			"a.txt":      "one\ntwo\n",
			"b.txt":      "1\n2\n3\n4\n5\n",
			"binary.bin": "a\xffb",
			"cut.bin":    "ab\xe2\x82",
			"other.txt":  "other\n",
			"dir/":       "",
			"first.txt":  "first\n",
			"second.txt": "second\n",
			"blocked":    "a file",
		},
		steps: []step{
			patch(""),
			patch("\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n"),
			patch("--- a/a.txt\n"),
			patch("+++ b/a.txt\n"),
			patch("@@ -1 +1 @@\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ nonsense\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -١ +١ @@\n-x\n+y\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -99999999999999999999 +1 @@\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -1,99999999999999999999 +1 @@\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -1 +99999999999999999999 @@\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\nnonsense\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n\\ No newline at end of file\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1 @@\n-one\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -1 +1,2 @@\n-one\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-wrong\n+x\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -9 +9 @@\n-wrong\n+x\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -0 +0,0 @@\n"),
			patch("--- a/b.txt\n+++ b/b.txt\n@@ -3,3 +3,0 @@\n-3\n-4\n-5\n@@ -1 +1 @@\n-1\n+x\n"),
			patch("--- a/a.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-one\n"),
			patch("--- a/a.txt\n+++ b/other.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+2\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+1\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+uno\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+1\n--- a/a.txt\n+++ b/./a.txt\n@@ -1 +1 @@\n-one\n+uno\n"),
			patch("--- a/binary.bin\n+++ b/binary.bin\n@@ -1 +1 @@\n-a\n+b\n"),
			patch("--- a/cut.bin\n+++ b/cut.bin\n@@ -1 +1 @@\n-a\n+b\n"),
			patch("--- a/missing.txt\n+++ b/missing.txt\n@@ -1 +1 @@\n-a\n+b\n"),
			patch("--- a/dir\n+++ b/dir\n@@ -1 +1 @@\n-a\n+b\n"),
			patch("--- a/\n+++ b/\n@@ -1 +1 @@\n-a\n+b\n"),
			patch("--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+1\n--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-first\n+changed\n--- a/second.txt\n+++ b/second.txt\n@@ -1 +1 @@\n-wrong\n+changed\n"),
			// The second write fails after the first: the first is restored.
			patch("--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-first\n+changed\n--- /dev/null\n+++ b/blocked/x/new.txt\n@@ -0,0 +1 @@\n+new\n"),
			patch("--- a/first.txt\n+++ b/first.txt\n@@ -1 +1 @@\n-first\n+changed\n--- /dev/null\n+++ b/blocked/new.txt\n@@ -0,0 +1 @@\n+new\n"),
			read("first.txt"),
			read("a.txt"),
		},
	},
	{
		name: "permissions are kept by edits and patches, and restored by a rollback",
		files: map[string]string{
			"exec.sh":     "echo\n",
			"private.txt": "secret\n",
			"shared.txt":  "shared\n",
			"gone.sh":     "gone\n",
			"real.txt":    "real\n",
			"link.txt":    "->real.txt",
			"blocked":     "a file",
		},
		modes: map[string]fs.FileMode{"exec.sh": 0o755, "private.txt": 0o600, "shared.txt": 0o664, "gone.sh": 0o700, "real.txt": 0o640},
		steps: []step{
			edit("private.txt", "secret", "changed", ""),
			patch("--- a/exec.sh\n+++ b/exec.sh\n@@ -1 +1 @@\n-echo\n+echo done\n"),
			edit("shared.txt", "shared", "edited", ""),
			create("created.txt", "new\n"),
			patch("--- a/link.txt\n+++ b/link.txt\n@@ -1 +1 @@\n-real\n+patched\n"),
			patch("--- a/gone.sh\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n--- a/exec.sh\n+++ b/exec.sh\n@@ -1 +1 @@\n-echo done\n+rolled back\n--- /dev/null\n+++ b/blocked/new.txt\n@@ -0,0 +1 @@\n+new\n"),
			patch("--- a/private.txt\n+++ b/moved-private.txt\n@@ -1 +1 @@\n-changed\n+moved\n"),
		},
	},
	{
		name:  "edits that undo each other leave no record, and edits of a file changed in between are separate",
		files: map[string]string{"x.txt": "a\n", "y.txt": "1\n"},
		steps: []step{
			edit("x.txt", "a", "b", ""),
			edit("x.txt", "b", "a", ""),
			edit("y.txt", "1", "2", ""),
			{operation: "file.edit", args: `{"path":"y.txt","old":"2","new":"3"}`, noEdits: true},
			edit("y.txt", "3", "4", ""),
			create("z.txt", "new\n"),
			{operation: "file.edit", args: `{"path":"z.txt","old":"new","new":"newer"}`, noEdits: true},
			edit("z.txt", "newer", "newest", ""),
			patch("--- a/z.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-newest\n"),
		},
	},
	{
		name:  "a record that is full of bytes stops copying contents",
		files: bigFiles(5, 7_000_000),
		steps: []step{
			edit("big0.txt", "END", "DONE", ""),
			edit("big1.txt", "END", "DONE", ""),
			edit("big2.txt", "END", "DONE", ""),
			edit("big3.txt", "END", "DONE", ""),
			edit("big4.txt", "END", "DONE", ""),
			create("after.txt", "still recorded\n"),
		},
	},
	{
		name:  "a record that lists as many files as it holds, and no more",
		steps: manyCreates(502),
	},
	{
		name:  "arguments that break their type",
		files: map[string]string{"a.txt": "a\n"},
		steps: []step{
			{operation: "file.read", args: `{}`, messageDiffers: "wire-contracts.md words a refusal by the field and the rule"},
			{operation: "file.read", args: `{"path":"a.txt","extra":1}`, messageDiffers: "wire-contracts.md words a refusal by the field and the rule"},
			{operation: "file.read", args: `{"path":null}`, messageDiffers: "wire-contracts.md words a refusal by the field and the rule"},
			{operation: "file.read", args: `{"path":1}`, messageDiffers: "wire-contracts.md words a refusal by the field and the rule"},
			{operation: "file.create", args: `{"path":"x"}`, messageDiffers: "wire-contracts.md words a refusal by the field and the rule"},
			{operation: "file.edit", args: `{"path":"a.txt","old":"","new":"b"}`, messageDiffers: "wire-contracts.md words a refusal by the field and the rule"},
			{operation: "file.edit", args: `{"path":"a.txt","old":"a","new":"b","occurrence":0}`, messageDiffers: "wire-contracts.md words a refusal by the field and the rule"},
			{operation: "file.edit", args: `{"path":"a.txt","old":"a","new":"b","context":null}`, messageDiffers: "wire-contracts.md words a refusal by the field and the rule"},
			{operation: "file.edit", args: `{"path":"a.txt","old":"a"}`, messageDiffers: "wire-contracts.md words a refusal by the field and the rule"},
			{operation: "file.patch", args: `{"patch":1}`, messageDiffers: "wire-contracts.md words a refusal by the field and the rule"},
			read("a.txt"),
		},
	},
}

// bigFiles returns count files of about size bytes of text, each ending in END.
func bigFiles(count, size int) map[string]string {
	files := map[string]string{}
	for i := range count {
		files[fmt.Sprintf("big%d.txt", i)] = strings.Repeat("a", size) + "\nEND\n"
	}
	return files
}

// manyCreates returns the steps that create count small files.
func manyCreates(count int) []step {
	steps := make([]step, count)
	for i := range steps {
		steps[i] = create(fmt.Sprintf("many/f%03d.txt", i), fmt.Sprintf("file %d\n", i))
	}
	return steps
}

func TestDifferentialTheFileOperationsAnswerAsTheRustProgramDoes(t *testing.T) {
	rust := os.Getenv("DEMI_DIFFTEST_RUST")
	if rust == "" {
		t.Skip("DEMI_DIFFTEST_RUST names the Rust demi-commands")
	}
	goProgram := os.Getenv("DEMI_DIFFTEST_GO")
	if goProgram == "" {
		goProgram = filepath.Join(t.TempDir(), "demi-commands")
		build := exec.Command("go", "build", "-o", goProgram, "github.com/wspl/demi/go/cmd/demi-commands")
		build.Env = append(os.Environ(), "CGO_ENABLED=0")
		if output, err := build.CombinedOutput(); err != nil {
			t.Fatalf("building the Go program: %v\n%s", err, output)
		}
	}
	for _, s := range table {
		t.Run(s.name, func(t *testing.T) {
			rustRun := play(t, rust, s)
			goRun := play(t, goProgram, s)
			compareRuns(t, s, rustRun, goRun)
		})
	}
}

func TestDifferentialBothProgramsListTheSameOperations(t *testing.T) {
	rust := os.Getenv("DEMI_DIFFTEST_RUST")
	if rust == "" {
		t.Skip("DEMI_DIFFTEST_RUST names the Rust demi-commands")
	}
	ctx, cancel := context.WithTimeout(context.Background(), hang)
	defer cancel()
	info, err := servicetest.StartProcess(ctx, t, rust, []string{"--command-service"}, nil).Client.Info(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(info.Operations, builtinproto.Operations()) {
		t.Errorf("the Rust program lists %v, the Go package %v", info.Operations, builtinproto.Operations())
	}
}

func TestDifferentialAProgramStartedWithoutItsFlagSaysHowToStartItAndExitsWith2(t *testing.T) {
	rust := os.Getenv("DEMI_DIFFTEST_RUST")
	if rust == "" {
		t.Skip("DEMI_DIFFTEST_RUST names the Rust demi-commands")
	}
	goProgram := os.Getenv("DEMI_DIFFTEST_GO")
	if goProgram == "" {
		t.Skip("DEMI_DIFFTEST_GO names the Go demi-commands")
	}
	for _, args := range [][]string{nil, {"--other"}, {"serve", "--command-service"}} {
		var outputs [2]string
		var codes [2]int
		for i, program := range []string{rust, goProgram} {
			var stdout, stderr bytes.Buffer
			cmd := exec.Command(program, args...)
			cmd.Stdout, cmd.Stderr = &stdout, &stderr
			err := cmd.Run()
			if exit, ok := err.(*exec.ExitError); ok {
				codes[i] = exit.ExitCode()
			} else if err != nil {
				t.Fatal(err)
			}
			outputs[i] = stdout.String() + "|" + stderr.String()
		}
		if codes[0] != codes[1] || outputs[0] != outputs[1] {
			t.Errorf("%v: the Rust program exits with %d and says %q, the Go program with %d and %q", args, codes[0], outputs[0], codes[1], outputs[1])
		}
	}
}

// play starts the program, and runs the scenario in a directory of its own.
func play(t *testing.T, program string, s scenario) run {
	t.Helper()
	root, err := filepath.EvalSymlinks(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	dir := filepath.Join(root, "work")
	if err := os.Mkdir(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	for name, content := range s.files {
		path := filepath.Join(dir, name)
		switch {
		case strings.HasSuffix(name, "/"):
			err = os.MkdirAll(path, 0o755)
		case strings.HasPrefix(content, "->"):
			err = os.Symlink(strings.TrimPrefix(content, "->"), path)
		default:
			if err = os.MkdirAll(filepath.Dir(path), 0o755); err == nil {
				err = os.WriteFile(path, []byte(content), 0o644)
			}
		}
		if err != nil {
			t.Fatal(err)
		}
	}
	for name, mode := range s.modes {
		if err := os.Chmod(filepath.Join(dir, name), mode); err != nil {
			t.Fatal(err)
		}
	}
	ctx, cancel := context.WithTimeout(context.Background(), hang)
	defer cancel()
	process := servicetest.StartProcess(ctx, t, program, []string{"--command-service"}, nil)
	edits := commandservice.EditContext{Directory: filepath.Join(root, "changes"), Lock: filepath.Join(root, "edits.lock")}
	substitute := func(text string) string {
		return strings.NewReplacer("{dir}", dir, "{root}", root).Replace(text)
	}
	normalize := func(text string) string {
		return strings.NewReplacer(dir, "{dir}", root, "{root}").Replace(text)
	}
	var result run
	for _, st := range s.steps {
		invocation := commandservice.Invocation{
			Operation:    st.operation,
			InvocationID: st.operation,
			Context: commandservice.CommandContext{
				Conversation: "file-test-conversation",
				Caller:       commandservice.AgentCaller{Number: 1},
				Locale:       commandservice.CommandLocale{TimeZone: "UTC", Languages: []string{"en-US"}},
			},
			Args: jsontext.Value(substitute(st.args)),
			Cwd:  dir,
			Env:  map[string]string{},
		}
		if !st.noEdits {
			invocation.Edits = &edits
		}
		stream, err := process.Client.Invoke(ctx, invocation)
		if err != nil {
			t.Fatalf("%s: %v", st.operation, err)
		}
		var stdout, stderr bytes.Buffer
		completion, err := commandservice.Exchange(ctx, stream, bytes.NewReader(nil), &stdout, &stderr)
		if err != nil {
			t.Fatalf("%s %s: %v", st.operation, st.args, err)
		}
		observed := observation{exitCode: int(completion.ExitCode), stdout: normalize(stdout.String()), stderr: normalize(stderr.String())}
		if completion.Error != nil {
			observed.errorCode = completion.Error.Code
			observed.message = normalize(completion.Error.Message)
		}
		result.steps = append(result.steps, observed)
	}
	result.tree = snapshot(t, root, dir, normalize)
	result.journal = journal(t, edits, normalize)
	return result
}

// snapshot describes every file under root, except the record of edits, which
// journal compares by what it means: its kind, its permissions and its content,
// or where a link leads.
func snapshot(t *testing.T, root, dir string, normalize func(string) string) map[string]string {
	t.Helper()
	tree := map[string]string{}
	err := filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		relative := normalize(path)
		if path == root || path == filepath.Join(root, "changes") || path == filepath.Join(root, "edits.lock") || strings.HasPrefix(path, filepath.Join(root, "changes")+string(filepath.Separator)) {
			return nil
		}
		info, err := os.Lstat(path)
		if err != nil {
			return err
		}
		switch {
		case info.Mode()&fs.ModeSymlink != 0:
			target, err := os.Readlink(path)
			if err != nil {
				return err
			}
			tree[relative] = "link to " + target
		case info.IsDir():
			tree[relative] = fmt.Sprintf("directory %v", info.Mode().Perm())
		default:
			data, err := os.ReadFile(path)
			if err != nil {
				return err
			}
			tree[relative] = fmt.Sprintf("file %v %q", info.Mode().Perm(), data)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	_ = dir
	return tree
}

// journal reads the record of the edits a program left, and describes what it
// means, the copies' contents included.
func journal(t *testing.T, edits commandservice.EditContext, normalize func(string) string) []string {
	t.Helper()
	if _, err := os.Stat(edits.Directory); err != nil {
		return []string{"no record"}
	}
	recorder, err := commandservice.NewRecorder(edits)
	if err != nil {
		t.Fatal(err)
	}
	report, err := recorder.Report()
	if err != nil {
		t.Fatalf("the record cannot be read: %v", err)
	}
	var lines []string
	for _, file := range report.Files {
		lines = append(lines, fmt.Sprintf("%s %s", normalize(file.Path), file.Kind))
		for _, segment := range file.Edits {
			original, modified := "none", "unavailable"
			if segment.Original != nil {
				data, err := os.ReadFile(*segment.Original)
				if err != nil {
					t.Fatal(err)
				}
				original = fmt.Sprintf("%q", data)
			}
			if segment.Modified != nil {
				data, err := os.ReadFile(*segment.Modified)
				if err != nil {
					t.Fatal(err)
				}
				modified = fmt.Sprintf("%q", data)
			}
			lines = append(lines, fmt.Sprintf("  from %s to %s", original, modified))
		}
	}
	lines = append(lines, fmt.Sprintf("truncated %v, %d bytes copied, %d segments", report.FilesTruncated, report.BytesCopied, report.NextSegment))
	return lines
}

func compareRuns(t *testing.T, s scenario, rust, goRun run) {
	t.Helper()
	for i, st := range s.steps {
		want, got := rust.steps[i], goRun.steps[i]
		if st.messageDiffers != "" {
			want.message, got.message = "", ""
		}
		if want != got {
			t.Errorf("step %d, %s %s\n  Rust: %+v\n  Go:   %+v", i+1, st.operation, abbreviate(st.args), want, got)
		}
		if testing.Verbose() {
			t.Logf("step %d, %s %s: exit %d %s %q, stdout %q", i+1, st.operation, abbreviate(st.args), got.exitCode, got.errorCode, got.message, abbreviate(got.stdout))
		}
	}
	names := map[string]bool{}
	for name := range rust.tree {
		names[name] = true
	}
	for name := range goRun.tree {
		names[name] = true
	}
	var sorted []string
	for name := range names {
		sorted = append(sorted, name)
	}
	sort.Strings(sorted)
	for _, name := range sorted {
		if rust.tree[name] != goRun.tree[name] {
			t.Errorf("%s\n  Rust: %s\n  Go:   %s", name, abbreviate(rust.tree[name]), abbreviate(goRun.tree[name]))
		}
	}
	if testing.Verbose() {
		for i, line := range goRun.journal {
			if i < 40 || i >= len(goRun.journal)-1 {
				t.Logf("journal: %s", abbreviate(line))
			}
		}
	}
	if !slices.Equal(rust.journal, goRun.journal) {
		t.Errorf("the record of edits differs\n  Rust: %q\n  Go:   %q", rust.journal, goRun.journal)
	}
}

// abbreviate shortens a long text for a report of a difference.
func abbreviate(text string) string {
	if len(text) > 240 {
		return text[:240] + "..."
	}
	return text
}
