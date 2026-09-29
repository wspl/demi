package shell

import (
	"errors"
	"io"
	"log/slog"
	"os"

	"github.com/wspl/demi/go/commandservice"
)

// FileChange is the completed job's edit entry. Counts use the runner's shared
// working-tree diff algorithm, supplied to ReportEdits by its owner in G5c.
type FileChange struct {
	commandservice.EditFile
	Added   uint64
	Removed uint64
}

// ReportEdits reads retained snapshots after Run has joined the job's writers.
// count is the runner's histogram line counter; nil before means an added file.
// A snapshot that cannot be read loses both sides, never substitutes live bytes.
func ReportEdits(recorder *commandservice.Recorder, count func(before, after []byte) (uint64, uint64)) ([]FileChange, bool) {
	if recorder == nil {
		return nil, false
	}
	journal, err := recorder.Report()
	if err != nil {
		slog.Warn("edit report failed", "error", err)
		return nil, false
	}
	files := make([]FileChange, 0, len(journal.Files))
	for _, file := range journal.Files {
		change := FileChange{EditFile: file}
		for i := range change.Edits {
			edit := &change.Edits[i]
			if edit.Modified == nil {
				continue
			}
			before, after, err := snapshots(edit)
			if err != nil {
				slog.Warn("edit snapshot read failed", "error", err)
				edit.Original = nil
				edit.Modified = nil
				continue
			}
			added, removed := count(before, after)
			change.Added += added
			change.Removed += removed
		}
		files = append(files, change)
	}
	return files, journal.FilesTruncated
}

// snapshots reads just the retained, bounded text sides of a recorded edit.
func snapshots(edit *commandservice.EditCopies) ([]byte, []byte, error) {
	var sides [2][]byte
	for i, path := range []*string{edit.Original, edit.Modified} {
		if path == nil {
			continue
		}
		file, err := os.Open(*path)
		if err != nil {
			return nil, nil, err
		}
		sides[i], err = io.ReadAll(io.LimitReader(file, commandservice.EditFileBytes+1))
		closeErr := file.Close()
		if err != nil {
			return nil, nil, err
		}
		if closeErr != nil {
			return nil, nil, closeErr
		}
		if len(sides[i]) > commandservice.EditFileBytes || !commandservice.IsText(sides[i]) {
			return nil, nil, errors.New("invalid edit snapshot")
		}
	}
	return sides[0], sides[1], nil
}
