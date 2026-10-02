package engine

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// editRecorder gives one shell job a journal while jobs share the same edit lock.
func editRecorder(t *testing.T, root, job string) *cmdsdk.Recorder {
	t.Helper()
	recorder, err := cmdsdk.NewRecorder(t.Context(), commandwire.EditContext{Directory: filepath.Join(root, job), Lock: filepath.Join(root, "edits.lock")})
	if err != nil {
		t.Fatal(err)
	}
	return recorder
}

// editJournal reads the persisted contract after the shell has joined its writers.
func editJournal(t *testing.T, recorder *cmdsdk.Recorder) commandwire.EditJournal {
	t.Helper()
	bytes, err := os.ReadFile(filepath.Join(recorder.Context().Directory, "journal.json"))
	if err != nil {
		t.Fatal(err)
	}
	journal, err := commandwire.DecodeEditJournal(bytes)
	if err != nil {
		t.Fatal(err)
	}
	return journal
}
func TestRedirectionsDescriptorsAndUtilitiesRecordActualContents(t *testing.T) {
	root := t.TempDir()
	recorder := editRecorder(t, root, "job")
	if err := os.WriteFile(filepath.Join(root, "restored"), []byte("same\n"), 0600); err != nil {
		t.Fatal(err)
	}
	// Utility-owned writes are explicitly deferred by the selected Go design.
	result, _, stderr := shellFiles(t, root, `printf 'one\n' > file; printf 'changed\n' > restored; printf 'same\n' > restored; exec 3>>file; printf 'two\n' >&3; exec 3>&-`, func(o *Options) { o.Edits = recorder })
	if result.Code != 0 {
		t.Fatalf("exit %d: %s", result.Code, stderr)
	}
	journal := editJournal(t, recorder)
	var changed []commandwire.EditFile
	for _, file := range journal.Files {
		if len(file.Edits) > 0 {
			changed = append(changed, file)
		}
	}
	if len(changed) != 1 || filepath.Base(changed[0].Path) != "file" || len(changed[0].Edits) != 1 {
		t.Fatalf("edits %+v", changed)
	}
	data, err := os.ReadFile(*changed[0].Edits[0].Modified)
	if err != nil || string(data) != "one\ntwo\n" {
		t.Fatalf("modified %q: %v", data, err)
	}
}
func TestRedirectedExternalOutputIsForwardedThroughTheRecorder(t *testing.T) {
	root := t.TempDir()
	recorder := editRecorder(t, root, "job")
	result, _, stderr := shellFiles(t, root, `/bin/sh -c 'printf child; printf error >&2' > out 2> err; cat out > observed; /bin/sh -c 'printf first; printf second >&2; printf third' > combined 2>&1; /bin/sh -c 'printf numbered >&3' 3> numbered`, func(o *Options) { o.Edits = recorder })
	if result.Code != 0 {
		t.Fatalf("exit %d: %s", result.Code, stderr)
	}
	want := map[string]string{"out": "child", "err": "error", "observed": "child", "combined": "firstsecondthird", "numbered": "numbered"}
	journal := editJournal(t, recorder)
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
	if err := os.WriteFile(filepath.Join(root, "file"), []byte("before\n"), 0600); err != nil {
		t.Fatal(err)
	}
	for _, step := range []struct {
		recorder *cmdsdk.Recorder
		script   string
	}{{a, "echo A > file"}, {b, "echo B > file"}, {a, "echo C > file"}} {
		result, _, stderr := shellFiles(t, root, step.script, func(o *Options) { o.Edits = step.recorder })
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
