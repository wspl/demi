package file

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"testing/synctest"

	"github.com/wspl/demi/internal/cmdpkg/file/fileop"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

func TestEditSelection(t *testing.T) {
	one, two, three, four := uint(1), uint(2), uint(3), uint(4)
	for _, test := range []struct {
		name                    string
		occurrence, line        *uint
		old, new, want, failure string
	}{
		{name: "ambiguous", old: "a", new: "b", failure: "Multiple matches in f; specify --occurrence or --context"},
		{name: "missing", old: "z", new: "b", failure: "No match found in f"},
		{name: "occurrence", occurrence: &two, old: "a", new: "b", want: "a\nx\nb\n"},
		{name: "out of range", occurrence: &three, old: "a", new: "b", failure: "Occurrence 3 is out of range"},
		{name: "nearest", line: &four, old: "a", new: "b", want: "a\nx\nb\n"},
		{name: "tie", line: &two, old: "a", new: "b", failure: "Context line 2 is ambiguous: occurrence 1 at line 1; occurrence 2 at line 3"},
		{name: "no context match", line: &one, old: "z", new: "b", failure: "No match found"},
		{name: "occurrence takes precedence", occurrence: &one, line: &three, old: "a", new: "b", want: "b\nx\na\n"},
		{name: "unchanged", occurrence: &one, old: "a", new: "a", want: "a\nx\na\n"},
	} {
		t.Run(test.name, func(t *testing.T) {
			cwd := t.TempDir()
			writeFixture(t, filepath.Join(cwd, "f"), "a\nx\na\n")
			_, err := edit(t.Context(), cwd, &fileop.EditArgs{Path: "f", Old: test.old, New: test.new, Occurrence: test.occurrence, Context: test.line}, nil)
			if test.failure != "" {
				if err == nil || err.Error() != test.failure {
					t.Fatalf("error=%v, want %s", err, test.failure)
				}
				test.want = "a\nx\na\n"
			} else if err != nil {
				t.Fatal(err)
			}
			if got := string(contents(t, filepath.Join(cwd, "f"))); got != test.want {
				t.Fatalf("got %q, want %q", got, test.want)
			}
		})
	}
}

func TestUnifiedDiffCases(t *testing.T) {
	for _, test := range []struct{ name, original, patch, want, failure string }{
		{"no newline", "old", "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n", "new", ""},
		{"header-like contents", "-- old\n", "--- a/f\n+++ b/f\n@@ -1 +1 @@\n--- old\n+++ new\n", "++ new\n", ""},
		{"offset", "a\nb\nc\n", "--- a/f\n+++ b/f\n@@ -1 +1,2 @@\n a\n+x\n@@ -3 +4 @@\n-c\n+d\n", "a\nx\nb\nd\n", ""},
		{"insertion", "a\n", "--- a/f\n+++ b/f\n@@ -1,0 +2 @@\n+b\n", "a\nb\n", ""},
		{"equivalent paths", "a\n", "--- a/./f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n", "b\n", ""},
		{"timestamps", "a\n", "--- a/f 2026-01-01 12:00:00 +0000\n+++ b/f\t2026-01-01\n@@ -1 +1 @@\n-a\n+b\n", "b\n", ""},
		{"counts", "a\n", "--- a/f\n+++ b/f\n@@ -1,2 +1 @@\n-a\n+b\n", "", "Patch does not apply to f: Patch hunk line counts do not match header"},
		{"mismatch", "a\n", "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-x\n+b\n", "", "Patch does not apply to f: Patch does not apply at line 1"},
		{"new first", "a\n", "+++ b/f\n", "", "New header precedes old header"},
		{"incomplete", "a\n", "--- a/f\n+++ b/f\n", "", "Invalid patch: missing file headers or hunks"},
		{"bad header", "a\n", "--- a/f\n+++ b/f\n@@ bad\n", "", "Invalid patch hunk header"},
		{"marker", "a\n", "--- a/f\n+++ b/f\n@@ -1 +1 @@\n\\ No newline at end of file\n", "", "Newline marker without patch line"},
	} {
		t.Run(test.name, func(t *testing.T) {
			cwd := t.TempDir()
			writeFixture(t, filepath.Join(cwd, "f"), test.original)
			_, err := applyPatch(t.Context(), cwd, test.patch, nil)
			if test.failure != "" {
				if err == nil || err.Error() != test.failure {
					t.Fatalf("error=%v, want %s", err, test.failure)
				}
				test.want = test.original
			} else if err != nil {
				t.Fatal(err)
			}
			if got := string(contents(t, filepath.Join(cwd, "f"))); got != test.want {
				t.Fatalf("got %q, want %q", got, test.want)
			}
		})
	}
}

func TestPatchRenameDeleteAndDuplicatePlanning(t *testing.T) {
	cwd := t.TempDir()
	writeFixture(t, filepath.Join(cwd, "old"), "a\n")
	_, err := applyPatch(t.Context(), cwd, "--- a/old\n+++ b/new\n@@ -1 +1 @@\n-a\n+b\n", nil)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(cwd, "old")); !errors.Is(err, os.ErrNotExist) {
		t.Fatal(err)
	}
	if string(contents(t, filepath.Join(cwd, "new"))) != "b\n" {
		t.Fatal("rename contents")
	}
	patch := "--- a/new\n+++ b/new\n@@ -1 +1 @@\n-b\n+c\n"
	if _, err := applyPatch(t.Context(), cwd, patch+patch, nil); err == nil || err.Error() != "Patch changes the same path more than once" {
		t.Fatal(err)
	}
	if string(contents(t, filepath.Join(cwd, "new"))) != "b\n" {
		t.Fatal("planning changed file")
	}
	if _, err := applyPatch(t.Context(), cwd, "--- a/new\n+++ /dev/null\n@@ -1 +0,0 @@\n-b\n", nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(cwd, "new")); !errors.Is(err, os.ErrNotExist) {
		t.Fatal(err)
	}
}

func TestMutationWaitCancellationReleasesAdmission(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		s := &service{}
		permit, err := s.mutations.Acquire(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		defer permit.Release()
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		done := make(chan error, 1)
		go func() {
			_, err := s.Invoke(ctx, cmdsdk.InvocationContext[commandwire.Invocation]{Request: commandwire.Invocation{Operation: "file.create", Args: []byte(`{"path":"unused","content":""}`)}})
			done <- err
		}()
		synctest.Wait()
		cancel()
		if err := <-done; !errors.Is(err, context.Canceled) {
			t.Fatal(err)
		}
		permit.Release()
		next := s.mutations.TryAcquire()
		if next == nil {
			t.Fatal("cancelled waiter retained admission")
		}
		next.Release()
	})
}

func TestEditPreservesSymlinkAndPermissions(t *testing.T) {
	cwd := t.TempDir()
	target := filepath.Join(cwd, "target")
	writeFixture(t, target, "before\n")
	link := filepath.Join(cwd, "link")
	if err := os.Symlink(target, link); err != nil {
		t.Skipf("symlink unavailable: %v", err)
	}
	before, err := os.Stat(target)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := edit(t.Context(), cwd, &fileop.EditArgs{Path: "link", Old: "before", New: "after"}, nil); err != nil {
		t.Fatal(err)
	}
	info, err := os.Lstat(link)
	if err != nil || info.Mode()&os.ModeSymlink == 0 {
		t.Fatalf("link=%v error=%v", info, err)
	}
	after, err := os.Stat(target)
	if err != nil {
		t.Fatal(err)
	}
	if after.Mode() != before.Mode() || string(contents(t, target)) != "after\n" {
		t.Fatal("edit lost target contents or permissions")
	}
}

func TestPatchRejectsNonTextBeforeMutation(t *testing.T) {
	for _, test := range []struct{ data, message string }{
		{"a\xff", "invalid utf-8 sequence of 1 bytes from index 1"},
		{"\xe2\x82x", "invalid utf-8 sequence of 2 bytes from index 0"},
		{"\xf0\x90\x80x", "invalid utf-8 sequence of 3 bytes from index 0"},
		{"a\xe2\x82", "incomplete utf-8 byte sequence from index 1"},
	} {
		t.Run(test.message, func(t *testing.T) {
			cwd := t.TempDir()
			path := filepath.Join(cwd, "f")
			writeFixture(t, path, test.data)
			_, err := applyPatch(t.Context(), cwd, "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-x\n+y\n", nil)
			if err == nil || err.Error() != test.message {
				t.Fatalf("got %v, want %s", err, test.message)
			}
			if string(contents(t, path)) != test.data {
				t.Fatal("invalid source changed")
			}
		})
	}
}
