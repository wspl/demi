package shelltest_test

import (
	"bytes"
	"context"
	"errors"
	"io"
	"os"
	"runtime"
	"testing"

	"github.com/wspl/demi/internal/runner/shell/shelltest"
)

func TestBuiltinPipelineEmitsBeforeInputEOF(t *testing.T) {
	root := t.TempDir()
	scope := shelltest.NewScope(t.Context(), nil)
	defer func() {
		scope.Cancel()
		scope.Finish(context.Background())
	}()
	input, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		_ = input.Close()
	}() // Cleanup also runs after cancellation closes the file.
	defer func() {
		_ = writer.Close()
	}() // Cleanup also runs after cancellation closes the file.
	reader, output, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		_ = reader.Close()
	}() // Cleanup also runs after cancellation closes the file.
	defer func() {
		_ = output.Close()
	}() // Cleanup also runs after cancellation closes the file.
	stderr, err := os.CreateTemp(root, "stderr")
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		_ = stderr.Close()
	}() // Cleanup also runs after cancellation closes the file.
	type outcome struct {
		result shelltest.Result
		err    error
	}
	done := make(chan struct{})
	var finished outcome
	go func() {
		result, err := shelltest.Execute(
			t.Context(),
			"cat | cat",
			shelltest.Options{
				Scope:  scope,
				Cwd:    root,
				Env:    map[string]string{"HOME": root, "PATH": os.Getenv("PATH")},
				Stdin:  input,
				Stdout: output,
				Stderr: stderr,
			},
		)
		finished = outcome{result, err}
		close(done)
	}()
	defer func() {
		scope.Cancel()
		<-done
	}()
	want := []byte{0, 255, 128, 10}
	if _, err := writer.Write(want); err != nil {
		t.Fatal(err)
	}
	got := make([]byte, len(want))
	if _, err := io.ReadFull(reader, got); err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, want) {
		t.Fatalf("output %v", got)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	<-done
	if finished.err != nil || finished.result.Code != 0 {
		t.Fatalf("result %+v: %v", finished.result, finished.err)
	}
}

func TestExecuteCancellationInterruptsBorrowedInput(t *testing.T) {
	ctx := t.Context()
	scope := shelltest.NewScope(ctx, nil)
	defer func() {
		scope.Cancel()
		scope.Finish(context.Background())
	}()
	input, writer, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		_ = input.Close()
		_ = writer.Close()
	}()
	reader, output, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		_ = reader.Close()
		_ = output.Close()
	}()
	done := make(chan error, 1)
	root := t.TempDir()
	go func() {
		script := "printf ready; read value"
		if runtime.GOOS != "windows" {
			script = "/bin/sh -c :; " + script
		}
		_, err := shelltest.Execute(
			ctx,
			script,
			shelltest.Options{Scope: scope, Cwd: root, Stdin: input, Stdout: output, Stderr: output},
		)
		done <- err
	}()
	ready := make([]byte, 5)
	if _, err := io.ReadFull(reader, ready); err != nil || string(ready) != "ready" {
		t.Fatalf("readiness %q: %v", ready, err)
	}
	if err := scope.Activity().WaitWaiting(ctx); err != nil {
		t.Fatal(err)
	}
	scope.Cancel()
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Fatalf("cancellation: %v", err)
	}
	if _, err := input.Stat(); err != nil {
		t.Fatalf("caller handle closed: %v", err)
	}
}
