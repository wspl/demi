package builtincommands

import (
	"bytes"
	"context"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/wspl/demi/go/commandservice"
)

func TestALaterWriteFailureRestoresEarlierFilesAndTheirRecords(t *testing.T) {
	root := t.TempDir()
	var changes []change
	for _, name := range []string{"first", "second"} {
		path := filepath.Join(root, name)
		data := []byte(name + "\n")
		if name == "first" {
			data = bytes.Repeat([]byte("x"), commandservice.EditFileBytes+1)
		}
		if err := os.WriteFile(path, data, 0o640); err != nil {
			t.Fatal(err)
		}
		permissions, err := permissionsOf(path)
		if err != nil {
			t.Fatal(err)
		}
		changes = append(changes, change{
			path:        path,
			before:      optionalBytes{present: true, data: data},
			after:       optionalBytes{present: true, data: []byte("changed\n")},
			permissions: &permissions,
		})
	}
	recorder, err := commandservice.NewRecorder(commandservice.EditContext{
		Directory: filepath.Join(root, "changes"),
		Lock:      filepath.Join(root, "edits.lock"),
	})
	if err != nil {
		t.Fatal(err)
	}
	recording := recorder.Begin()
	for _, c := range changes {
		recording.Track(c.path)
	}
	simulated := errors.New("simulated write failure")
	err = commitChanges(context.Background(), changes, func(c change) error {
		if filepath.Base(c.path) == "second" {
			return simulated
		}
		return writeChange(c)
	}, recording)
	recording.Close()
	if err != simulated {
		t.Errorf("the failure = %v, want the write's own", err)
	}
	// The first file holds what it held, permissions included, and the journal
	// has nothing of it: it was restored, so it did not change.
	if got, _ := os.ReadFile(changes[0].path); !bytes.Equal(got, changes[0].before.data) {
		t.Errorf("the first file was not restored: %d bytes", len(got))
	}
	if info, err := os.Stat(changes[0].path); err != nil || info.Mode().Perm() != 0o640 {
		t.Errorf("the first file's permissions = %v, %v", info.Mode(), err)
	}
	if got, _ := os.ReadFile(changes[1].path); string(got) != "second\n" {
		t.Errorf("the second file = %q", got)
	}
	report, err := recorder.Report()
	if err != nil || len(report.Files) != 0 {
		t.Errorf("the journal = %+v, %v; want no file", report, err)
	}
}

func TestAFailureToRestoreIsReportedAfterTheFailureThatCausedIt(t *testing.T) {
	root := t.TempDir()
	blocked := filepath.Join(root, "blocked")
	if err := os.Mkdir(blocked, 0o755); err != nil {
		t.Fatal(err)
	}
	// The file to restore is now a directory, which cannot be replaced by a file.
	changes := []change{
		{path: blocked, before: optionalBytes{present: true, data: []byte("before")}, after: optionalBytes{present: true, data: []byte("after")}},
		{path: filepath.Join(root, "second"), before: optionalBytes{present: true, data: []byte("s")}, after: optionalBytes{present: true, data: []byte("t")}},
	}
	err := commitChanges(context.Background(), changes, func(c change) error {
		if c.path == blocked {
			return nil
		}
		return errors.New("write failed")
	}, nil)
	if err == nil || !strings.HasPrefix(err.Error(), "write failed\nRollback failed: ") {
		t.Fatalf("the failure = %v", err)
	}
	if errors.Is(err, errCancelled) {
		t.Error("a failure with a report of a rollback is a cancellation")
	}
}

func TestACancelledCommitRestoresWhatItWroteAndIsACancellation(t *testing.T) {
	root := t.TempDir()
	first, second := filepath.Join(root, "first"), filepath.Join(root, "second")
	for _, path := range []string{first, second} {
		if err := os.WriteFile(path, []byte("before"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	changes := []change{
		{path: first, before: optionalBytes{present: true, data: []byte("before")}, after: optionalBytes{present: true, data: []byte("after")}},
		{path: second, before: optionalBytes{present: true, data: []byte("before")}, after: optionalBytes{present: true, data: []byte("after")}},
	}
	ctx, cancel := context.WithCancel(context.Background())
	err := commitChanges(ctx, changes, func(c change) error {
		err := writeChange(c)
		// The invocation is cancelled after the first write.
		cancel()
		return err
	}, nil)
	if !errors.Is(err, errCancelled) {
		t.Fatalf("the failure = %v, want a cancellation", err)
	}
	for _, path := range []string{first, second} {
		if got, _ := os.ReadFile(path); string(got) != "before" {
			t.Errorf("%s = %q, want what it held", path, got)
		}
	}
}

func TestInvalidUTF8IsDescribedAsRustDescribesIt(t *testing.T) {
	for _, test := range []struct{ data, want string }{
		{"", ""},
		{"héllo € \U0001F600", ""},
		{"\xff", "invalid utf-8 sequence of 1 bytes from index 0"},
		{"abc\xc0\x80", "invalid utf-8 sequence of 1 bytes from index 3"},
		{"ab\xe2\x28\xa1", "invalid utf-8 sequence of 1 bytes from index 2"},
		{"\xe2\x82\x28", "invalid utf-8 sequence of 2 bytes from index 0"},
		{"\xf0\x9f\x98\x28", "invalid utf-8 sequence of 3 bytes from index 0"},
		{"\xf0\x9f\x28", "invalid utf-8 sequence of 2 bytes from index 0"},
		{"\xed\xa0\x80", "invalid utf-8 sequence of 1 bytes from index 0"},
		{"\xf4\x90\x80\x80", "invalid utf-8 sequence of 1 bytes from index 0"},
		{"\xc3", "incomplete utf-8 byte sequence from index 0"},
		{"abc\xe2\x82", "incomplete utf-8 byte sequence from index 3"},
		{"a\xf0\x9f\x98", "incomplete utf-8 byte sequence from index 1"},
	} {
		got := ""
		if err := checkUTF8([]byte(test.data)); err != nil {
			got = err.Error()
		}
		if got != test.want {
			t.Errorf("%q: %q, want %q", test.data, got, test.want)
		}
	}
}

func TestAPathNameThatMoreThanOneWritingNamesIsOnePath(t *testing.T) {
	a, b := "/work/a.txt", "/work/./a.txt"
	c := "/work/sub/../a.txt"
	if !samePath(&a, &b) || samePath(&a, &c) || samePath(&a, nil) || !samePath(nil, nil) {
		t.Error("paths are compared by their components, and a .. is not resolved")
	}
}
