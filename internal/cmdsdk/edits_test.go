package cmdsdk

import (
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/commandwire"
)

func recorder(t *testing.T, root string) *Recorder {
	t.Helper()
	r, err := NewRecorder(t.Context(), commandwire.EditContext{Directory: filepath.Join(root, "job"), Lock: filepath.Join(root, "edits.lock")})
	must(t, err)
	return r
}
func snapshot(t *testing.T, path *string) string {
	t.Helper()
	if path == nil {
		return ""
	}
	b, err := os.ReadFile(*path)
	must(t, err)
	return string(b)
}
func report(t *testing.T, r *Recorder) commandwire.EditJournal {
	t.Helper()
	j, err := r.Report(t.Context())
	must(t, err)
	return j
}
func TestSeparateRecorderHandlesShareJobJournal(t *testing.T) {
	root := t.TempDir()
	first := recorder(t, root)
	native := recorder(t, root)
	path := filepath.Join(root, "file")
	must(t, first.Record(t.Context(), path, func() error { return os.WriteFile(path, []byte("one"), 0600) }))
	must(t, native.Record(t.Context(), path, func() error { return os.WriteFile(path, []byte("two"), 0600) }))
	j := report(t, first)
	if len(j.Files) != 1 || j.Files[0].Kind != commandwire.EditAdded || len(j.Files[0].Edits) != 1 {
		t.Fatalf("journal: %+v", j)
	}
	edit := j.Files[0].Edits[0]
	if snapshot(t, edit.Original) != "" || snapshot(t, edit.Modified) != "two" {
		t.Fatal("wrong snapshots")
	}
}
func TestErrorCanLeaveRealEdit(t *testing.T) {
	root := t.TempDir()
	r := recorder(t, root)
	path := filepath.Join(root, "file")
	failed := errors.New("failed after writing")
	err := r.Record(t.Context(), path, func() error {
		if err := os.WriteFile(path, []byte("partial"), 0600); err != nil {
			return err
		}
		return failed
	})
	if !errors.Is(err, failed) {
		t.Fatal(err)
	}
	if snapshot(t, report(t, r).Files[0].Edits[0].Modified) != "partial" {
		t.Fatal("partial edit lost")
	}
}
func TestFailedOpenOfLargeFileIsNotEdit(t *testing.T) {
	root := t.TempDir()
	r := recorder(t, root)
	path := filepath.Join(root, "large")
	f, err := os.Create(path)
	must(t, err)
	must(t, f.Truncate(commandwire.EditFileBytes+1))
	must(t, f.Close())
	err = r.Record(t.Context(), path, func() error {
		f, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
		if err == nil {
			return f.Close()
		}
		return err
	})
	if !errors.Is(err, os.ErrExist) {
		t.Fatal(err)
	}
	if len(report(t, r).Files) != 0 {
		t.Fatal("failed open invented edit")
	}
	must(t, r.Record(t.Context(), path, func() error {
		f, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0600)
		if err != nil {
			return err
		}
		_, err = f.Write([]byte("x"))
		return errors.Join(err, f.Close())
	}))
	j := report(t, r)
	if len(j.Files) != 1 || j.Files[0].Edits[0].Modified != nil {
		t.Fatalf("journal: %+v", j)
	}
}
func TestFailedWriteBelowFileIsNotEdit(t *testing.T) {
	root := t.TempDir()
	r := recorder(t, root)
	path := filepath.Join(root, "file")
	must(t, os.WriteFile(path, []byte("plain"), 0600))
	below := filepath.Join(path, "child")
	if err := r.Record(t.Context(), below, func() error { return os.WriteFile(below, []byte("never"), 0600) }); err == nil {
		t.Fatal("write succeeded")
	}
	if len(report(t, r).Files) != 0 {
		t.Fatal("invented edit")
	}
}
func TestBinaryEditsHaveNoContents(t *testing.T) {
	root := t.TempDir()
	r := recorder(t, root)
	path := filepath.Join(root, "binary")
	must(t, r.Record(t.Context(), path, func() error { return os.WriteFile(path, []byte{0, 1, 2}, 0600) }))
	j := report(t, r)
	if len(j.Files) != 1 || j.Files[0].Kind != commandwire.EditAdded || j.Files[0].Edits[0].Modified != nil {
		t.Fatalf("journal: %+v", j)
	}
}
func TestEditContinuityAndRestoration(t *testing.T) {
	root := t.TempDir()
	r := recorder(t, root)
	path := filepath.Join(root, "file")
	write := func(value string) {
		t.Helper()
		must(t, r.Record(t.Context(), path, func() error { return os.WriteFile(path, []byte(value), 0600) }))
	}
	must(t, os.WriteFile(path, []byte("original"), 0600))
	write("one")
	write("original")
	if len(report(t, r).Files) != 0 {
		t.Fatal("restored bytes retained a diff")
	}
	write("two")
	must(t, os.WriteFile(path, []byte("external"), 0600))
	write("three")
	j := report(t, r)
	if len(j.Files[0].Edits) != 2 {
		t.Fatalf("lost separate edit segment: %+v", j)
	}
	g := r.Begin(t.Context())
	if g == nil {
		t.Fatal("begin")
	}
	g.Track(t.Context(), path)
	g.Restored(path)
	g.Close(t.Context())
	must(t, r.Record(t.Context(), path, func() error { return os.Remove(path) }))
	if len(report(t, r).Files) != 0 {
		t.Fatal("deleted path retained")
	}
}

// Cost: one local recording and journal read; no subprocesses or timed waits.
func TestJournalPreservesSerdeStringBytes(t *testing.T) {
	root := t.TempDir()
	r := recorder(t, root)
	name := "file&\u2028\u2029"
	if runtime.GOOS != "windows" {
		name += "<>"
	}
	path := filepath.Join(root, name)
	must(t, r.Record(t.Context(), path, func() error { return os.WriteFile(path, []byte("one"), 0600) }))
	data, err := os.ReadFile(filepath.Join(r.Context().Directory, "journal.json"))
	must(t, err)
	if !strings.Contains(string(data), name) {
		t.Fatalf("journal escaped path: %s", data)
	}
}
