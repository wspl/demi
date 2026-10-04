package engine_test

import (
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
	"github.com/wspl/demi/internal/runner/shell/internal/engine"
)

// editRecorder gives one shell job a journal while jobs share the same edit lock.
func editRecorder(t *testing.T, root, job string) *commandsdk.Recorder {
	t.Helper()
	recorder, err := commandsdk.NewRecorder(
		t.Context(),
		commandproto.EditContext{Directory: filepath.Join(root, job), Lock: filepath.Join(root, "edits.lock")},
	)
	if err != nil {
		t.Fatal(err)
	}
	return recorder
}

// editJournal observes the job's public report after its writers have joined.
func editJournal(t *testing.T, recorder *commandsdk.Recorder) commandproto.EditJournal {
	t.Helper()
	journal, err := recorder.Report(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	return journal
}

func TestRedirectionsDescriptorsAndUtilitiesRecordActualContents(t *testing.T) {
	t.Skip("decision 4: system utility writes are absent from the edit report")
	root := t.TempDir()
	recorder := editRecorder(t, root, "job")
	for name, contents := range map[string]string{"sorted": "pear\napple\n", "restored": "same\n"} {
		if err := os.WriteFile(filepath.Join(root, name), []byte(contents), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	result, _, stderr := shellFiles(
		t,
		root,
		`printf 'one\n' > file; printf 'changed\n' > restored; `+
			`printf 'same\n' > restored; exec 3>>file; printf 'two\n' >&3; exec 3>&-; `+
			`printf 'tea\n' | tee tee-file >/dev/null; sort sorted -o sorted; `+
			`sed -i 's/one/first/' file; printf 'same\nsame\n' | uniq - unique; `+
			`printf temporary > temporary; rm temporary; cp file copy; mv copy moved; `+
			`touch touched; mktemp >/dev/null`,
		func(o *engine.Options) {
			o.Edits = recorder
			o.Env["TMPDIR"] = root
		},
	)
	if result.Code != 0 {
		t.Fatalf("exit %d: %s", result.Code, stderr)
	}
	journal := editJournal(t, recorder)
	var names []string
	for _, file := range journal.Files {
		names = append(names, filepath.Base(file.Path))
	}
	if !reflect.DeepEqual(names, []string{"file", "tee-file", "sorted", "unique"}) {
		t.Fatalf("reported files %v", names)
	}
	for _, file := range journal.Files {
		name := filepath.Base(file.Path)
		want := map[string]string{
			"file":     "first\ntwo\n",
			"tee-file": "tea\n",
			"sorted":   "apple\npear\n",
			"unique":   "same\n",
		}[name]
		if len(file.Edits) != 1 || file.Edits[0].Modified == nil {
			t.Fatalf("%s edits %+v", name, file.Edits)
		}
		data, err := os.ReadFile(*file.Edits[0].Modified)
		if err != nil || string(data) != want {
			t.Fatalf("%s modified %q, want %q: %v", name, data, want, err)
		}
		if name == "sorted" {
			if file.Edits[0].Original == nil {
				t.Fatal("sorted original missing")
			}
			data, err = os.ReadFile(*file.Edits[0].Original)
			if err != nil || string(data) != "pear\napple\n" {
				t.Fatalf("sorted original %q: %v", data, err)
			}
		}
	}
}

func TestRedirectedExternalOutputIsForwardedThroughTheRecorder(t *testing.T) {
	root := t.TempDir()
	recorder := editRecorder(t, root, "job")
	result, _, stderr := shellFiles(
		t,
		root,
		`/bin/sh -c 'printf child; printf error >&2' > out 2> err; cat out > observed; `+
			`/bin/sh -c 'printf first; printf second >&2; printf third' > combined 2>&1; `+
			`/bin/sh -c 'printf numbered >&3' 3> numbered`,
		func(o *engine.Options) { o.Edits = recorder },
	)
	if result.Code != 0 {
		t.Fatalf("exit %d: %s", result.Code, stderr)
	}
	want := map[string]string{
		"out":      "child",
		"err":      "error",
		"observed": "child",
		"combined": "firstsecondthird",
		"numbered": "numbered",
	}
	journal := editJournal(t, recorder)
	if len(journal.Files) != len(want) {
		t.Fatalf("files %+v", journal.Files)
	}
	for _, file := range journal.Files {
		name := filepath.Base(file.Path)
		if len(file.Edits) != 1 || file.Edits[0].Modified == nil {
			t.Fatalf("edits %+v", file)
		}
		bytes, err := os.ReadFile(*file.Edits[0].Modified)
		if err != nil || string(bytes) != want[name] {
			t.Fatalf("%s: %q want %q (%v)", name, bytes, want[name], err)
		}
		delete(want, name)
	}
	if len(want) > 0 {
		t.Fatalf("missing edits: %v", want)
	}
}

func TestAnotherJobCannotChangeAnAlreadyCapturedAfterSide(t *testing.T) {
	root := t.TempDir()
	a, b := editRecorder(t, root, "a"), editRecorder(t, root, "b")
	if err := os.WriteFile(filepath.Join(root, "file"), []byte("before\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	for _, step := range []struct {
		recorder *commandsdk.Recorder
		script   string
	}{{a, "echo A > file"}, {b, "echo B > file"}, {a, "echo C > file"}} {
		result, _, stderr := shellFiles(t, root, step.script, func(o *engine.Options) { o.Edits = step.recorder })
		if result.Code != 0 {
			t.Fatalf("exit %d: %s", result.Code, stderr)
		}
	}
	journal := editJournal(t, a)
	if len(journal.Files) != 1 || len(journal.Files[0].Edits) != 2 {
		t.Fatalf("journal %+v", journal)
	}
	for index, want := range [][2]string{{"before\n", "A\n"}, {"B\n", "C\n"}} {
		edit := journal.Files[0].Edits[index]
		for side, path := range []*string{edit.Original, edit.Modified} {
			if path == nil {
				t.Fatal("missing snapshot")
			}
			data, err := os.ReadFile(*path)
			if err != nil || string(data) != want[side] {
				t.Fatalf("snapshot %q want %q (%v)", data, want[side], err)
			}
		}
	}
}
