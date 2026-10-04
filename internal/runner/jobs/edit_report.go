package jobs

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"os"

	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerproto"
)

// finishEdits reports retained snapshots rather than attributing subsequent workspace writes.
func finishEdits(ctx context.Context, recorder *commandsdk.Recorder) ([]runnerproto.JobFileChange, bool) {
	files := []runnerproto.JobFileChange{}
	if recorder == nil {
		return files, false
	}
	journal, err := recorder.Report(ctx)
	if err != nil {
		slog.Warn("edit report failed", "error", err)
		return files, false
	}
	for _, file := range journal.Files {
		var added, removed uint64
		for i := range file.Edits {
			edit := &file.Edits[i]
			if edit.Modified == nil {
				continue
			}
			var original []byte
			var err error
			if edit.Original != nil {
				original, err = readEditSnapshot(*edit.Original)
			}
			var modified []byte
			if err == nil {
				modified, err = readEditSnapshot(*edit.Modified)
			}
			if err != nil {
				slog.Warn("edit snapshot read failed", "error", err)
				edit.Original = nil
				edit.Modified = nil
				continue
			}
			a, r := process.LineCounts(original, modified)
			added += a
			removed += r
		}
		files = append(
			files,
			runnerproto.JobFileChange{
				Path:    file.Path,
				Kind:    file.Kind,
				Edits:   file.Edits,
				Added:   added,
				Removed: removed,
			},
		)
	}
	return files, journal.FilesTruncated
}

// readEditSnapshot enforces the recorder's retained-text bound before diffing it.
func readEditSnapshot(path string) ([]byte, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	// Cleanup follows the operation result; cancellation may already have closed it.
	defer func() { _ = file.Close() }()
	bytes, err := io.ReadAll(io.LimitReader(file, commandproto.EditFileBytes+1))
	if err != nil {
		return nil, err
	}
	if len(bytes) > commandproto.EditFileBytes || !commandproto.IsText(bytes) {
		return nil, errors.New("invalid edit snapshot")
	}
	return bytes, nil
}
