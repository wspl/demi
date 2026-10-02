package artifacts

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// ReleaseFile describes one verified file and its path inside a release.
type ReleaseFile struct {
	Source, Path string
	Digest       Digest
	Executable   bool
}

// ReleaseRecord is the caller-owned release descriptor, encoded by its contract.
type ReleaseRecord struct {
	Name  string
	Bytes []byte
}

// ConflictError identifies an immutable release that differs from this one.
type ConflictError struct{ Path string }

func (e *ConflictError) Error() string {
	return fmt.Sprintf("%s is already published with other contents", e.Path)
}

// PublishRelease stages all verified files and the record before publishing the
// whole immutable directory. Repeating the same publication checks its contents.
func PublishRelease(ctx context.Context, directory string, record ReleaseRecord, files []ReleaseFile) (err error) {
	if !insideArchive(record.Name) || strings.Contains(record.Name, "/") {
		return fmt.Errorf("release record must be one file name: %w", fs.ErrInvalid)
	}
	names := map[string]bool{record.Name: true}
	for _, file := range files {
		name := filepath.ToSlash(file.Path)
		if !insideArchive(name) || names[name] {
			return fmt.Errorf("invalid or duplicate release path %q: %w", file.Path, fs.ErrInvalid)
		}
		names[name] = true
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	parent := filepath.Dir(directory)
	if parent == "." || filepath.Base(directory) == "." {
		return fmt.Errorf("release directory needs a parent: %w", fs.ErrInvalid)
	}
	if err := os.MkdirAll(parent, 0755); err != nil {
		return err
	}
	stage, err := os.MkdirTemp(parent, "."+filepath.Base(directory)+"-stage-")
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, os.RemoveAll(stage)) }()
	directories := map[string]bool{stage: true}
	for _, file := range files {
		destination := filepath.Join(stage, file.Path)
		for d := filepath.Dir(destination); d != stage; d = filepath.Dir(d) {
			directories[d] = true
		}
		if err := stageReleaseFile(ctx, file, destination); err != nil {
			return err
		}
	}
	if err := PublishBytes(ctx, filepath.Join(stage, record.Name), record.Bytes, Publication{Durable: true}); err != nil {
		return err
	}
	ordered := make([]string, 0, len(directories))
	for d := range directories {
		ordered = append(ordered, d)
	}
	sort.Slice(ordered, func(i, j int) bool { return len(ordered[i]) > len(ordered[j]) })
	for _, d := range ordered {
		if err := syncDirectory(ctx, d); err != nil {
			return err
		}
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	if err := os.Rename(stage, directory); err != nil {
		info, statErr := os.Stat(directory)
		if statErr == nil && info.IsDir() {
			return releaseInPlace(ctx, directory, record, files)
		}
		return err
	}
	return syncDirectory(context.WithoutCancel(ctx), parent)
}
func stageReleaseFile(ctx context.Context, file ReleaseFile, destination string) (err error) {
	if err := os.MkdirAll(filepath.Dir(destination), 0755); err != nil {
		return err
	}
	input, err := os.Open(file.Source)
	if err != nil {
		return err
	}
	defer func() { _ = input.Close() }() // Read-only source.
	// The release directory is already private staging: individual files need
	// no second temporary or directory sync before the whole stage is committed.
	output, err := os.OpenFile(destination, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0666)
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, output.Close()) }()
	if err := Copy(ctx, input, file.Digest, output); err != nil {
		return err
	}
	if file.Executable {
		if err := output.Chmod(0755); err != nil {
			return err
		}
	}
	return output.Sync()
}
func releaseInPlace(ctx context.Context, directory string, record ReleaseRecord, files []ReleaseFile) error {
	name := filepath.Join(directory, record.Name)
	data, err := os.ReadFile(name)
	if errors.Is(err, os.ErrNotExist) || (err == nil && !bytes.Equal(data, record.Bytes)) {
		return &ConflictError{name}
	}
	if err != nil {
		return err
	}
	for _, file := range files {
		name := filepath.Join(directory, file.Path)
		found, err := DigestFile(ctx, name, file.Digest.Size)
		var tooLarge *TooLargeError
		if errors.Is(err, os.ErrNotExist) || errors.As(err, &tooLarge) || (err == nil && found != file.Digest) {
			return &ConflictError{name}
		}
		if err != nil {
			return err
		}
	}
	return nil
}
