package tools_test

import (
	"context"
	"errors"
	"os"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/wspl/demi/go/machines/internal/tools"
)

func TestAMissingProgramIsNamed(t *testing.T) {
	t.Setenv("PATH", t.TempDir())
	_, err := tools.Resolve("/nonexistent/runsc")
	var missing *tools.MissingError
	if !errors.As(err, &missing) {
		t.Fatalf("%v", err)
	}
	want := "Cloud manager needs: mke2fs, e2fsck, resize2fs, bsdtar, nft, runsc"
	if err.Error() != want {
		t.Errorf("%q, want %q", err, want)
	}
}

func TestATailIsCutAtACharacterBoundary(t *testing.T) {
	if got := tools.Tail("abcdef", 3); got != "def" {
		t.Errorf("%q", got)
	}
	if got := tools.Tail("abc", 10); got != "abc" {
		t.Errorf("%q", got)
	}
	// Two bytes of a three-byte character cannot start the tail.
	if got := tools.Tail("a€b", 3); got != "b" || !utf8.ValidString(got) {
		t.Errorf("%q", got)
	}
}

func TestAProgramsFailureCarriesTheEndOfItsOutput(t *testing.T) {
	programs, err := tools.Resolve(os.Args[0])
	if err != nil {
		t.Skip(err)
	}
	long := strings.Repeat("x", 10000)
	// The tools run as programs of the manager; the shell stands in for one.
	shell := tools.Bsdtar
	programs = tools.WithProgram(programs, shell, "/bin/sh")
	ctx := context.Background()
	output, err := programs.Run(ctx, shell, []string{"-c", "echo out; echo problem >&2; exit 3"}, 0)
	var failed *tools.Error
	if !errors.As(err, &failed) || failed.Kind != tools.Failed || output.Code != 0 {
		t.Fatalf("%+v, %v", output, err)
	}
	if want := "bsdtar exited 3: problem"; err.Error() != want {
		t.Errorf("%q, want %q", err, want)
	}
	// Without error output, the standard output is the message; both are bounded.
	_, err = programs.Run(ctx, shell, []string{"-c", "echo " + long + "; exit 1"}, 0)
	if !strings.HasSuffix(err.Error(), long[:8*1024]) || len(err.Error()) > 8*1024+64 {
		t.Errorf("a long message of %d bytes", len(err.Error()))
	}
	// An exit status the caller accepts is not a failure.
	output, err = programs.Output(ctx, shell, []string{"-c", "exit 1"}, 0)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := tools.Accept(shell, output, 0, 1); err != nil {
		t.Errorf("exit 1 accepted: %v", err)
	}
	if _, err := tools.Accept(shell, output, 0); err == nil {
		t.Error("exit 1 refused")
	}
	// A program a signal ended has no status.
	_, err = programs.Run(ctx, shell, []string{"-c", "kill -9 $$"}, 0)
	if err == nil || !strings.HasPrefix(err.Error(), "bsdtar exited by signal") {
		t.Errorf("%v", err)
	}
	// One that cannot start says so.
	missing := tools.WithProgram(programs, shell, "/nonexistent/program")
	if _, err := missing.Run(ctx, shell, nil, 0); err == nil || !strings.HasPrefix(err.Error(), "bsdtar could not start") {
		t.Errorf("%v", err)
	}
}
