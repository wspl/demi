package jobs_test

import (
	"bytes"
	"io"
	"os"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/internal/runner/jobs"
	"github.com/wspl/demi/internal/runnerproto"
)

// This file-only scenario writes 32 MiB and uses no external programs.
func TestKeptSnapshotSurvivesTailRotation(t *testing.T) {
	ctx := t.Context()
	directory := filepath.Join(t.TempDir(), "kept")
	output, err := jobs.CreateKeptOutput(ctx, directory)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := output.Close(); err != nil {
			t.Error(err)
		}
	})
	chunk := bytes.Repeat([]byte("a"), 64*1024)
	for range 192 {
		if err := output.Write(ctx, runnerproto.Stdout, chunk); err != nil {
			t.Fatal(err)
		}
	}
	snapshot, err := output.Reader().Snapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := snapshot.Close(); err != nil {
			t.Error(err)
		}
	}()
	reference, err := output.Reader().Snapshot(ctx)
	if err != nil {
		t.Fatal(err)
	}
	want, err := io.ReadAll(reference)
	if closeErr := reference.Close(); closeErr != nil {
		t.Fatal(closeErr)
	}
	if err != nil {
		t.Fatal(err)
	}
	for range 320 {
		if err := output.Write(ctx, runnerproto.Stdout, chunk); err != nil {
			t.Fatal(err)
		}
	}
	if err := output.Close(); err != nil {
		t.Fatal(err)
	}
	got, err := io.ReadAll(snapshot)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(got, want) {
		t.Fatal("snapshot changed after retained tail files rotated")
	}
	entries, err := os.ReadDir(directory)
	if err != nil {
		t.Fatal(err)
	}
	var size int64
	for _, entry := range entries {
		info, err := entry.Info()
		if err != nil {
			t.Fatal(err)
		}
		size += info.Size()
	}
	if size > 2*runnerproto.JobKeptPartBytes {
		t.Fatalf("open snapshot left %d retained bytes, beyond the bound", size)
	}
}
