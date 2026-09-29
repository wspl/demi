package commandservice_test

import (
	"errors"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/commandservice"
)

// recorder returns a recorder of the job named job under root; every recorder
// under one root shares its lock.
func recorder(t *testing.T, root, job string) *commandservice.Recorder {
	t.Helper()
	recorder, err := commandservice.NewRecorder(commandservice.EditContext{
		Directory: filepath.Join(root, job),
		Lock:      filepath.Join(root, "edits.lock"),
	})
	if err != nil {
		t.Fatal(err)
	}
	return recorder
}

// contents reads the two sides of one segment of an edit.
func contents(t *testing.T, edit commandservice.EditCopies) (original, modified string) {
	t.Helper()
	if edit.Original != nil {
		data, err := os.ReadFile(*edit.Original)
		if err != nil {
			t.Fatal(err)
		}
		original = string(data)
	}
	data, err := os.ReadFile(*edit.Modified)
	if err != nil {
		t.Fatal(err)
	}
	return original, string(data)
}

func write(path string, data string) func() error {
	return func() error { return os.WriteFile(path, []byte(data), 0o644) }
}

func TestSeparateRecorderHandlesShareTheJobJournal(t *testing.T) {
	root := t.TempDir()
	first := recorder(t, root, "job")
	native := recorder(t, root, "job")
	path := filepath.Join(root, "file")
	if err := first.Record(path, write(path, "one")); err != nil {
		t.Fatal(err)
	}
	if err := native.Record(path, write(path, "two")); err != nil {
		t.Fatal(err)
	}
	report, err := first.Report()
	if err != nil {
		t.Fatal(err)
	}
	if len(report.Files) != 1 || report.Files[0].Kind != commandservice.EditAdded || len(report.Files[0].Edits) != 1 {
		t.Fatalf("the report = %+v, want one added file with one segment", report)
	}
	// Both writes are one segment of a file the job added: nothing before it,
	// what the second write left after it.
	if original, modified := contents(t, report.Files[0].Edits[0]); original != "" || modified != "two" {
		t.Errorf("the segment = %q to %q, want nothing to %q", original, modified, "two")
	}
}

func TestAnEditThatFailsAfterWritingIsStillARealEdit(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	path := filepath.Join(root, "file")
	err := recorder.Record(path, func() error {
		if err := os.WriteFile(path, []byte("partial"), 0o644); err != nil {
			return err
		}
		return errors.New("failed after writing")
	})
	if err == nil {
		t.Fatal("the operation's error was lost")
	}
	report, err := recorder.Report()
	if err != nil {
		t.Fatal(err)
	}
	if _, modified := contents(t, report.Files[0].Edits[0]); modified != "partial" {
		t.Errorf("the segment ends with %q, want %q", modified, "partial")
	}
}

func TestAFileOverTheSnapshotLimitIsRecordedWithoutContentsOnlyWhenItChanges(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	path := filepath.Join(root, "large")
	file, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := file.Truncate(commandservice.EditFileBytes + 1); err != nil {
		t.Fatal(err)
	}
	file.Close()
	// An operation that fails to open the file changes nothing: no edit.
	err = recorder.Record(path, func() error {
		created, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o644)
		if err == nil {
			created.Close()
		}
		return err
	})
	if err == nil {
		t.Fatal("the file exists, and its creation succeeded")
	}
	report, err := recorder.Report()
	if err != nil {
		t.Fatal(err)
	}
	if len(report.Files) != 0 {
		t.Fatalf("an operation that changed nothing left %+v", report)
	}
	// One that appends a byte does: a modified file whose segment has no contents.
	err = recorder.Record(path, func() error {
		appended, err := os.OpenFile(path, os.O_WRONLY|os.O_APPEND, 0)
		if err != nil {
			return err
		}
		defer appended.Close()
		_, err = appended.WriteString("x")
		return err
	})
	if err != nil {
		t.Fatal(err)
	}
	report, err = recorder.Report()
	if err != nil {
		t.Fatal(err)
	}
	if len(report.Files) != 1 || report.Files[0].Edits[0].Modified != nil {
		t.Errorf("the report = %+v, want one file whose segment has no contents", report)
	}
}

func TestABinaryFileIsRecordedWithoutContents(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	path := filepath.Join(root, "binary")
	if err := recorder.Record(path, func() error { return os.WriteFile(path, []byte{0, 1, 2}, 0o644) }); err != nil {
		t.Fatal(err)
	}
	report, err := recorder.Report()
	if err != nil {
		t.Fatal(err)
	}
	if report.Files[0].Kind != commandservice.EditAdded || report.Files[0].Edits[0].Modified != nil {
		t.Errorf("the report = %+v, want an added file without contents", report)
	}
}

func TestAFileThatIsGoneIsLeftOutOfTheReport(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	path := filepath.Join(root, "file")
	if err := recorder.Record(path, write(path, "text")); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	report, err := recorder.Report()
	if err != nil {
		t.Fatal(err)
	}
	if len(report.Files) != 0 {
		t.Errorf("a removed file is in the report: %+v", report)
	}
}

func TestARecordingThatCannotBeginLetsTheOperationRun(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	// The job's lock cannot be opened: its name is a directory.
	if err := os.Mkdir(filepath.Join(root, "edits.lock"), 0o755); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(root, "file")
	if err := recorder.Record(path, write(path, "text")); err != nil {
		t.Fatalf("the operation failed: %v", err)
	}
	if data, err := os.ReadFile(path); err != nil || string(data) != "text" {
		t.Errorf("the file = %q, %v", data, err)
	}
}

func TestAJournalThatPointsOutsideItsJobIsRefused(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	journal := `{"files":[{"path":"/x","kind":"added","edits":[{"modified":"` + filepath.Join(root, "other", "0.modified") + `"}]}],` +
		`"bytesCopied":0,"nextSegment":1,"filesTruncated":false}`
	if err := os.WriteFile(filepath.Join(root, "job", "journal.json"), []byte(journal), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := recorder.Report(); err == nil || !strings.Contains(err.Error(), "outside its job directory") {
		t.Errorf("the report of a journal that points elsewhere: %v", err)
	}
}

func TestARecorderRefusesAContextThatIsNotAbsolute(t *testing.T) {
	_, err := commandservice.NewRecorder(commandservice.EditContext{Directory: "job", Lock: "/lock"})
	if err == nil || err.Error() != "invalid: directory: is not an absolute path" {
		t.Errorf("error = %v", err)
	}
}

func TestAPathIsResolvedAgainstTheDirectoryAnInvocationNames(t *testing.T) {
	for name, test := range map[string]struct{ cwd, path, want string }{
		"a relative path":          {"/work", "a/b", "/work/a/b"},
		"a directory with a slash": {"/work/", "a", "/work/a"},
		"an absolute path":         {"/work", "/etc/a", "/etc/a"},
		"a path that goes up":      {"/work/link", "../x", "/work/link/../x"},
		"a path that stays":        {"/work", "./a", "/work/./a"},
		"no directory":             {"", "a", "a"},
		"the root":                 {"/", "a", "/a"},
		"a name with a backslash":  {"/work", `a\b`, `/work/a\b`},
		"a directory that is dots": {"/work", "..", "/work/.."},
	} {
		got, err := commandservice.ResolvePath(test.cwd, test.path)
		if err != nil || got != test.want {
			t.Errorf("%s: %q, %v, want %q", name, got, err, test.want)
		}
	}
	for _, path := range []string{"", "a\x00b"} {
		if _, err := commandservice.ResolvePath("/work", path); !errors.Is(err, commandservice.ErrInvalidPath) {
			t.Errorf("the path %q: %v", path, err)
		}
	}
	if got := commandservice.ErrInvalidPath.Error(); got != "path must be nonempty and contain no NUL byte" {
		t.Errorf("the message = %q", got)
	}
}

func TestTwoPathsAreTheSameWhenTheirComponentsAre(t *testing.T) {
	for _, same := range [][2]string{
		{"/a/b", "/a/./b"},
		{"/a//b", "/a/b"},
		{"/a/b/", "/a/b"},
		{"./a/./b", "./a/b"},
		{"a/b/.", "a/b"},
	} {
		if commandservice.PathKey(same[0]) != commandservice.PathKey(same[1]) {
			t.Errorf("%q and %q differ", same[0], same[1])
		}
	}
	// A ".." is left alone: it may name another place through a link.
	for _, different := range [][2]string{
		{"/a/b", "/a/c/../b"},
		{"/a/b", "a/b"},
		{"/a/b", "/a/B"},
		{"./a", "a"},
	} {
		if commandservice.PathKey(different[0]) == commandservice.PathKey(different[1]) {
			t.Errorf("%q and %q are the same", different[0], different[1])
		}
	}
}

func TestAnOperationThatFindsNoDescriptorWaitsWithABackoffAndTriesAgain(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		attempts := 0
		start := time.Now()
		value, err := commandservice.RetryBlocking(func() (string, error) {
			attempts++
			if attempts <= 7 {
				return "", &os.PathError{Op: "open", Path: "x", Err: commandservice.ErrNoDescriptor}
			}
			return "opened", nil
		})
		if err != nil || value != "opened" || attempts != 8 {
			t.Fatalf("%q, %v after %d attempts", value, err, attempts)
		}
		// The pauses double from 5 ms to at most 100 ms.
		want := (5 + 10 + 20 + 40 + 80 + 100 + 100) * time.Millisecond
		if waited := time.Since(start); waited != want {
			t.Errorf("waited %s, want %s", waited, want)
		}
	})
}

func TestAnOperationThatFailsForAnotherReasonIsNotRetried(t *testing.T) {
	attempts := 0
	_, err := commandservice.RetryBlocking(func() (int, error) {
		attempts++
		return 0, os.ErrPermission
	})
	if !errors.Is(err, os.ErrPermission) || attempts != 1 {
		t.Errorf("%v after %d attempts", err, attempts)
	}
	if !commandservice.Exhausted(&os.PathError{Err: commandservice.ErrNoDescriptor}) || commandservice.Exhausted(os.ErrNotExist) {
		t.Error("only a lack of open files is exhaustion")
	}
}

func TestTheParentOfAPathIsItsComponentsWithoutTheLast(t *testing.T) {
	for path, want := range map[string]string{
		"/a/b":    "/a",
		"/a/b/":   "/a",
		"/a":      "/",
		"a/b":     "a",
		"a":       "",
		"/a/../b": "/a/..",
		"./a":     ".",
		"a/./b":   "a",
	} {
		got, ok := commandservice.ParentPath(path)
		if !ok || got != want {
			t.Errorf("the parent of %q = %q, %v, want %q", path, got, ok, want)
		}
	}
	for _, path := range []string{"/", "", "//"} {
		if got, ok := commandservice.ParentPath(path); ok {
			t.Errorf("the parent of %q = %q, want none", path, got)
		}
	}
}

func TestEditsThatUndoEachOtherLeaveNoRecordButAFileChangedInBetweenKeepsBoth(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	path := filepath.Join(root, "file")
	if err := os.WriteFile(path, []byte("a\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	for _, text := range []string{"b\n", "a\n"} {
		if err := recorder.Record(path, write(path, text)); err != nil {
			t.Fatal(err)
		}
	}
	report, err := recorder.Report()
	if err != nil || len(report.Files) != 0 {
		t.Fatalf("a file that ended as it began: %+v, %v", report, err)
	}
	// A change nobody recorded splits what follows into a segment of its own.
	if err := recorder.Record(path, write(path, "b\n")); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte("c\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := recorder.Record(path, write(path, "d\n")); err != nil {
		t.Fatal(err)
	}
	report, err = recorder.Report()
	if err != nil || len(report.Files) != 1 || len(report.Files[0].Edits) != 2 {
		t.Fatalf("the report = %+v, %v; want one file with two segments", report, err)
	}
	if original, modified := contents(t, report.Files[0].Edits[1]); original != "c\n" || modified != "d\n" {
		t.Errorf("the second segment = %q to %q", original, modified)
	}
}

// Cost: 0.1 s. The limit is 500 files, so the test tracks 502 in one recording;
// no smaller record reaches it.
func TestARecordThatIsFullOfFilesSaysSoAndListsNoMore(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	// One recording tracks all the files before it changes them, as an operation
	// that changes many does.
	recording := recorder.Begin()
	var paths []string
	for i := range commandservice.EditJobFiles + 2 {
		path := filepath.Join(root, "file"+itoa(i))
		paths = append(paths, path)
		recording.Track(path)
	}
	for _, path := range paths {
		if err := os.WriteFile(path, []byte("text"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	recording.Close()
	report, err := recorder.Report()
	if err != nil {
		t.Fatal(err)
	}
	if len(report.Files) != commandservice.EditJobFiles || !report.FilesTruncated {
		t.Errorf("the report lists %d files, truncated %v", len(report.Files), report.FilesTruncated)
	}
}

// Cost: 0.6 s (1.3 s under -race). The limit is 1,000 segments, and a segment
// needs a change nobody recorded between two recorded ones, so the test makes
// three rounds of 500 files; no smaller record reaches the limit.
func TestARecordThatIsFullOfSegmentsKeepsMetadataForTheFilesThatNoLongerFit(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	var paths []string
	for i := range commandservice.EditJobFiles {
		paths = append(paths, filepath.Join(root, "file"+itoa(i)))
	}
	// Each round changes every file without a record, and then with one: the
	// change nobody recorded makes what follows a segment of its own. Two rounds
	// hold as many segments as the record does; the third does not fit.
	for round := range 3 {
		for _, path := range paths {
			if err := os.WriteFile(path, []byte("unrecorded "+itoa(round)), 0o644); err != nil {
				t.Fatal(err)
			}
		}
		recording := recorder.Begin()
		for _, path := range paths {
			recording.Track(path)
		}
		for _, path := range paths {
			if err := os.WriteFile(path, []byte("recorded "+itoa(round)), 0o644); err != nil {
				t.Fatal(err)
			}
		}
		recording.Close()
	}
	report, err := recorder.Report()
	if err != nil {
		t.Fatal(err)
	}
	if !report.FilesTruncated || report.NextSegment != commandservice.EditJobSegments {
		t.Errorf("truncated %v after %d segments", report.FilesTruncated, report.NextSegment)
	}
	for _, file := range report.Files {
		if len(file.Edits) != 1 || file.Edits[0].Modified != nil || file.Edits[0].Original != nil {
			t.Fatalf("%s: %+v; want the record of a file whose segments no longer fit: one, without contents", file.Path, file.Edits)
		}
	}
}

// Cost: about a second (three under -race). The limit is 64 MiB of copies, so a
// record reaches it only by copying that much; no cheaper test proves that a
// full record stops copying and keeps listing files.
func TestARecordThatIsFullOfBytesStopsCopyingContents(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	// Each edit copies the file before and after it: two copies of 7 MB, and the
	// fifth would go over the record's 64 MiB.
	text := strings.Repeat("a", 7_000_000) + "\nEND\n"
	recording := recorder.Begin()
	var paths []string
	for i := range 5 {
		path := filepath.Join(root, "big"+itoa(i))
		if err := os.WriteFile(path, []byte(text), 0o644); err != nil {
			t.Fatal(err)
		}
		paths = append(paths, path)
		recording.Track(path)
	}
	for _, path := range paths {
		if err := os.WriteFile(path, []byte(strings.Replace(text, "END", "DONE", 1)), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	recording.Close()
	report, err := recorder.Report()
	if err != nil || len(report.Files) != 5 {
		t.Fatalf("the report = %v files, %v", len(report.Files), err)
	}
	for _, file := range report.Files[:4] {
		if file.Edits[0].Modified == nil {
			t.Errorf("%s has no contents, and the record was not full", file.Path)
		}
	}
	if last := report.Files[4].Edits[0]; last.Modified != nil || last.Original != nil {
		t.Errorf("the edit that did not fit has contents: %+v", last)
	}
	if report.BytesCopied > commandservice.EditJobBytes {
		t.Errorf("%d bytes were copied, over the limit", report.BytesCopied)
	}
}

func itoa(n int) string { return strconv.Itoa(n) }

func TestAWriteBelowAFileIsNotAnEdit(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	file := filepath.Join(root, "file")
	if err := os.WriteFile(file, []byte("text"), 0o644); err != nil {
		t.Fatal(err)
	}
	// The write fails, since a file is not a directory, and writes nothing: a path
	// below a file names nothing, so it is absent before and after.
	path := filepath.Join(file, "below.txt")
	if err := recorder.Record(path, write(path, "x")); err == nil {
		t.Fatal("a write below a file succeeded")
	}
	report, err := recorder.Report()
	if err != nil || len(report.Files) != 0 {
		t.Errorf("the report = %+v, %v; want no file", report, err)
	}
}

// A file over the snapshot limit has no contents to compare, so two states of it
// are told apart by their size and times; one replaced by another of the same
// size and modification time differs by the time it was created.
func TestAFileReplacedByAnotherOfTheSameSizeAndModificationTimeIsAChange(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	path := filepath.Join(root, "large")
	modified := time.Date(2026, 1, 2, 3, 4, 5, 0, time.UTC)
	create := func() {
		file, err := os.Create(path)
		if err != nil {
			t.Fatal(err)
		}
		if err := file.Truncate(commandservice.EditFileBytes + 1); err != nil {
			t.Fatal(err)
		}
		file.Close()
		if err := os.Chtimes(path, modified, modified); err != nil {
			t.Fatal(err)
		}
	}
	create()
	born, known := commandservice.BirthTime(path)
	if !known {
		t.Skip("this file system does not tell when a file was created")
	}
	recording := recorder.Begin()
	recording.Track(path)
	// Files made in the same clock tick have one creation time; make another until
	// its time differs.
	for {
		if err := os.Remove(path); err != nil {
			t.Fatal(err)
		}
		create()
		if again, _ := commandservice.BirthTime(path); !again.Equal(born) {
			break
		}
	}
	recording.Close()
	report, err := recorder.Report()
	if err != nil || len(report.Files) != 1 || report.Files[0].Kind != commandservice.EditModified {
		t.Errorf("the report = %+v, %v; want one modified file", report, err)
	}
}

// Cost: about a second (three under -race): eight files of 8 MiB, which the record's 64 MiB holds to
// the byte.
func TestARecordHoldsExactlyItsLimitOfBytes(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	text := strings.Repeat("a", commandservice.EditFileBytes)
	recording := recorder.Begin()
	var paths []string
	for i := range commandservice.EditJobBytes / commandservice.EditFileBytes {
		path := filepath.Join(root, "new"+itoa(i))
		paths = append(paths, path)
		recording.Track(path)
	}
	for _, path := range paths {
		if err := os.WriteFile(path, []byte(text), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	recording.Close()
	report, err := recorder.Report()
	if err != nil || len(report.Files) != len(paths) {
		t.Fatalf("the report = %v files, %v", len(report.Files), err)
	}
	for _, file := range report.Files {
		if file.Edits[0].Modified == nil {
			t.Errorf("%s has no contents, and the copies just fit", file.Path)
		}
	}
	if report.BytesCopied != commandservice.EditJobBytes {
		t.Errorf("%d bytes were copied, want the limit %d", report.BytesCopied, commandservice.EditJobBytes)
	}
}

func TestAPathThatBecameADirectoryIsLeftOutOfTheReport(t *testing.T) {
	root := t.TempDir()
	recorder := recorder(t, root, "job")
	path := filepath.Join(root, "file")
	if err := recorder.Record(path, write(path, "text")); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(path, 0o755); err != nil {
		t.Fatal(err)
	}
	report, err := recorder.Report()
	if err != nil || len(report.Files) != 0 {
		t.Errorf("the report = %+v, %v; want no file", report, err)
	}
}
