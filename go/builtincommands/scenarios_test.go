package builtincommands_test

// The scenarios of the differential judge (difftest_test.go), and what does not
// need the Rust program: a scenario runs in a directory of its own through the
// client of a program, and what the program answered and left is a run.
//
// Volatile values are normalized by one rule: the paths of the run (the root
// directory of a program's run, and its working directory) are replaced by {root}
// and {dir}, in what a program answers, in the files it leaves and in the
// journal of its edits. Nothing else is: bytes, exit codes, error codes, error
// messages and the files' contents and permissions are compared as they are.

import (
	"bytes"
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"testing"

	"github.com/wspl/demi/go/builtincommands"
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
	// journalDiffers says why the journal of edits is not compared with the
	// Rust program's: the Rust program of the judge tree predates the fix that
	// treats a path below a file as absent (1484ef93), so it records a failed
	// write below a file as an edit. The Go tests pin the fixed behavior.
	journalDiffers string
}

// An observation is what a program answered to one invocation.
type observation struct {
	ExitCode  int
	ErrorCode string
	Message   string
	Stdout    string
	Stderr    string
}

// A run is what a program did with a scenario.
type run struct {
	Steps   []observation
	Tree    map[string]string
	Journal []string
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

func edit(path, old, replacement string, extra string) step {
	return step{operation: "file.edit", args: `{"path":` + quote(path) + `,"old":` + quote(old) + `,"new":` + quote(replacement) + extra + `}`}
}

func patch(text string) step {
	return step{operation: "file.patch", args: `{"patch":` + quote(text) + `}`}
}

// play runs the scenario in a directory of its own, through the client of a
// program.
func play(t *testing.T, client *commandservice.Client, s scenario) run {
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
		stream, err := client.Invoke(ctx, invocation)
		if err != nil {
			t.Fatalf("%s: %v", st.operation, err)
		}
		var stdout, stderr bytes.Buffer
		completion, err := commandservice.Exchange(ctx, stream, bytes.NewReader(nil), &stdout, &stderr)
		if err != nil {
			t.Fatalf("%s %s: %v", st.operation, st.args, err)
		}
		observed := observation{ExitCode: int(completion.ExitCode), Stdout: normalize(stdout.String()), Stderr: normalize(stderr.String())}
		if completion.Error != nil {
			observed.ErrorCode = completion.Error.Code
			observed.Message = normalize(completion.Error.Message)
		}
		result.Steps = append(result.Steps, observed)
	}
	result.Tree = snapshot(t, root, dir, normalize)
	result.Journal = journal(t, edits, normalize)
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

// abbreviate shortens a long text for a report of a difference.
func abbreviate(text string) string {
	if len(text) > 240 {
		return text[:240] + "..."
	}
	return text
}

// The behaviors below are pinned as data taken from the Rust program
// (testdata/rust-observations.json), so that they are held when the Rust
// program is gone. The differential judge records them:
//
//	DEMI_DIFFTEST_RUST=<demi-commands> DEMI_DIFFTEST_RECORD=testdata/rust-observations.json \
//	    go test -tags difftest -run DifferentialRecord ./builtincommands
var pinned = []scenario{
	{
		name: "the tie rule of --context: the nearest occurrence wins unless another is as near",
		files: map[string]string{
			"lines.txt": "x\nx\nx\nx\nx\n",
			"pair.txt":  "t\nm\nt\n",
			"far.txt":   "t\nm\nm\nm\nt\n",
		},
		steps: []step{
			edit("lines.txt", "x", "y", `,"context":3`),
			edit("lines.txt", "x", "y", `,"context":4`),
			edit("lines.txt", "x", "y", `,"context":5`),
			edit("lines.txt", "x", "y", `,"context":100`),
			read("lines.txt"),
			edit("pair.txt", "t", "T", `,"context":2`),
			edit("pair.txt", "t", "T", `,"context":1`),
			read("pair.txt"),
			edit("far.txt", "t", "T", `,"context":3`),
			edit("far.txt", "t", "T", `,"context":2`),
			read("far.txt"),
		},
	},
	{
		name:  "the upper bound of --occurrence is the number of matches",
		files: map[string]string{"two.txt": "a\na\n", "one.txt": "a\n"},
		steps: []step{
			edit("two.txt", "a", "b", `,"occurrence":2`),
			edit("two.txt", "b", "c", `,"occurrence":2`),
			edit("two.txt", "a", "x", `,"occurrence":2`),
			edit("two.txt", "b", "x", `,"occurrence":3`),
			edit("one.txt", "a", "b", `,"occurrence":2`),
			edit("one.txt", "a", "b", `,"occurrence":1`),
			read("two.txt"),
			read("one.txt"),
		},
	},
	{
		name: "a rollback restores the contents and the permissions of the files it wrote and the one it deleted",
		files: map[string]string{
			"exec.sh": "echo\n",
			"gone.sh": "gone\n",
			"secret":  "secret\n",
			"linkdir": "->nowhere",
		},
		modes: map[string]fs.FileMode{"exec.sh": 0o755, "gone.sh": 0o700, "secret": 0o600},
		steps: []step{
			patch("--- a/gone.sh\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n--- a/exec.sh\n+++ b/exec.sh\n@@ -1 +1 @@\n-echo\n+changed\n--- a/secret\n+++ b/secret\n@@ -1 +1 @@\n-secret\n+changed\n--- /dev/null\n+++ b/linkdir/new.txt\n@@ -0,0 +1 @@\n+new\n"),
			read("exec.sh"),
		},
	},
}

// The pinned behaviors are those of the Rust program: what the Go program
// answers and leaves is what the Rust program did.
func TestThePinnedBehaviorsAreThoseOfTheRustProgram(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("testdata", "rust-observations.json"))
	if err != nil {
		t.Fatal(err)
	}
	var recorded map[string]run
	if err := json.Unmarshal(data, &recorded); err != nil {
		t.Fatal(err)
	}
	if len(recorded) != len(pinned) {
		t.Errorf("%d scenarios are recorded and %d are pinned", len(recorded), len(pinned))
	}
	for _, s := range pinned {
		t.Run(s.name, func(t *testing.T) {
			want, ok := recorded[s.name]
			if !ok {
				t.Fatal("the Rust program's observations are not recorded")
			}
			compareRuns(t, s, want, play(t, servicetest.Start(t, builtincommands.New()).Client, s))
		})
	}
}

// compareRuns reports how the two runs of a scenario differ.
func compareRuns(t *testing.T, s scenario, rust, goRun run) {
	t.Helper()
	for i, st := range s.steps {
		want, got := rust.Steps[i], goRun.Steps[i]
		if st.messageDiffers != "" {
			want.Message, got.Message = "", ""
		}
		if want != got {
			t.Errorf("step %d, %s %s\n  Rust: %+v\n  Go:   %+v", i+1, st.operation, abbreviate(st.args), want, got)
		}
		if testing.Verbose() {
			t.Logf("step %d, %s %s: exit %d %s %q, stdout %q", i+1, st.operation, abbreviate(st.args), got.ExitCode, got.ErrorCode, got.Message, abbreviate(got.Stdout))
		}
	}
	names := map[string]bool{}
	for name := range rust.Tree {
		names[name] = true
	}
	for name := range goRun.Tree {
		names[name] = true
	}
	var sorted []string
	for name := range names {
		sorted = append(sorted, name)
	}
	sort.Strings(sorted)
	for _, name := range sorted {
		if rust.Tree[name] != goRun.Tree[name] {
			t.Errorf("%s\n  Rust: %s\n  Go:   %s", name, abbreviate(rust.Tree[name]), abbreviate(goRun.Tree[name]))
		}
	}
	if testing.Verbose() {
		for i, line := range goRun.Journal {
			if i < 40 || i >= len(goRun.Journal)-1 {
				t.Logf("journal: %s", abbreviate(line))
			}
		}
	}
	if s.journalDiffers == "" && !slices.Equal(rust.Journal, goRun.Journal) {
		t.Errorf("the record of edits differs\n  Rust: %q\n  Go:   %q", rust.Journal, goRun.Journal)
	}
}
