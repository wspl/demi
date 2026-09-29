package artifact

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"slices"
	"strings"
)

// A ReleaseFile is one file of a release: where its bytes are read from, its
// path inside the release directory, the size and SHA-256 it must have, and
// whether it is a program.
type ReleaseFile struct {
	Source     string
	Path       string
	Digest     Digest
	Executable bool
}

// A ReleaseRecord is the file that describes a release, such as a package
// descriptor: its name in the release directory and its bytes.
type ReleaseRecord struct {
	Name  string
	Bytes []byte
}

// PublishRelease publishes a directory of verified files and the record that
// describes them, once (docs/delivery/builds-and-releases.md § Packaging).
// Every file is copied into a stage beside the directory and checked against
// its declared size and SHA-256 as it is copied, and the stage becomes the
// directory in one rename. A release is immutable: one already in place is
// accepted only when its record and every file are the ones being published,
// and anything else there is a [*ConflictError]. However publication ends, its
// stage is gone. Once ctx is done, nothing is published.
func PublishRelease(ctx context.Context, directory string, record ReleaseRecord, files []ReleaseFile) error {
	if !isFileName(record.Name) {
		return invalidInput("a release record is one file name")
	}
	if !allInside(files) {
		return invalidInput("a release file's path stays inside its release")
	}
	clean := filepath.Clean(directory)
	parent, name := filepath.Dir(clean), filepath.Base(clean)
	if parent == clean || !strings.ContainsRune(clean, filepath.Separator) {
		return invalidInput("a release directory needs a parent directory")
	}
	if err := os.MkdirAll(parent, 0o777); err != nil {
		return err
	}
	stage, err := os.MkdirTemp(parent, "."+name+"-stage-")
	if err != nil {
		return err
	}
	published := false
	defer func() {
		if !published {
			// The stage is not the release; nothing else refers to it.
			os.RemoveAll(stage)
		}
	}()
	directories := []string{stage}
	for _, file := range files {
		// The directories of the file are those of its own path, from the stage
		// down; the path is plain (allInside), so they are the stage's own.
		components := pathComponents(file.Path)
		destination := filepath.Join(append([]string{stage}, components...)...)
		for i := 1; i < len(components); i++ {
			directories = append(directories, filepath.Join(append([]string{stage}, components[:i]...)...))
		}
		if err := stageFile(ctx, file, destination); err != nil {
			return err
		}
	}
	if err := writeRecord(filepath.Join(stage, record.Name), record.Bytes); err != nil {
		return err
	}
	// Deepest first, so each directory's entries are on disk before its
	// parent's; equal paths end up side by side for Compact.
	slices.SortFunc(directories, func(a, b string) int {
		if depth := strings.Count(b, string(filepath.Separator)) - strings.Count(a, string(filepath.Separator)); depth != 0 {
			return depth
		}
		return strings.Compare(a, b)
	})
	for _, dir := range slices.Compact(directories) {
		if err := syncDirectory(dir); err != nil {
			return err
		}
	}
	if err := ctx.Err(); err != nil {
		return err
	}
	if err := os.Rename(stage, directory); err != nil {
		// A directory in place fails the rename, with an error that differs by
		// platform; it is the release only when it holds the same bytes.
		if info, statErr := os.Stat(directory); statErr == nil && info.IsDir() {
			return inPlace(ctx, directory, record, files)
		}
		return err
	}
	// The stage is the release now; there is nothing left to remove.
	published = true
	return syncDirectory(parent)
}

func invalidInput(reason string) error {
	return fmt.Errorf("%w: %s", fs.ErrInvalid, reason)
}

// isFileName reports whether name is one plain file name.
func isFileName(name string) bool {
	return name != "" && name != "." && name != ".." && !strings.ContainsAny(name, `/\`) && filepath.IsLocal(name)
}

func allInside(files []ReleaseFile) bool {
	return !slices.ContainsFunc(files, func(file ReleaseFile) bool {
		return !plainPath(file.Path)
	})
}

// pathComponents splits a path of a release file at "/" and at the backslashes
// of a Windows path.
func pathComponents(path string) []string {
	return strings.Split(strings.ReplaceAll(path, `\`, "/"), "/")
}

// plainPath reports whether every component of path is a plain name: not empty,
// not "." and not "..".
func plainPath(path string) bool {
	for _, component := range pathComponents(path) {
		if component == "" || component == "." || component == ".." {
			return false
		}
	}
	return filepath.IsLocal(filepath.FromSlash(path))
}

// stageFile copies file to destination, checking its bytes as they are copied.
func stageFile(ctx context.Context, file ReleaseFile, destination string) error {
	if err := os.MkdirAll(filepath.Dir(destination), 0o777); err != nil {
		return err
	}
	input, err := os.Open(file.Source)
	if err != nil {
		return err
	}
	defer input.Close()
	output, err := os.Create(destination)
	if err != nil {
		return err
	}
	defer output.Close()
	if err := Copy(ctx, input, file.Digest, output); err != nil {
		return err
	}
	if err := output.Sync(); err != nil {
		return err
	}
	if file.Executable {
		// A program is runnable whatever the umask.
		return os.Chmod(destination, 0o755)
	}
	return nil
}

func writeRecord(path string, bytes []byte) error {
	file, err := os.Create(path)
	if err != nil {
		return err
	}
	defer file.Close()
	if _, err := file.Write(bytes); err != nil {
		return err
	}
	return file.Sync()
}

// inPlace accepts the release at directory when its record and files are the
// ones being published; anything else there is a conflict.
func inPlace(ctx context.Context, directory string, record ReleaseRecord, files []ReleaseFile) error {
	path := filepath.Join(directory, record.Name)
	found, err := os.ReadFile(path)
	switch {
	case errors.Is(err, fs.ErrNotExist):
		return &ConflictError{Path: path}
	case err != nil:
		return err
	case !bytes.Equal(found, record.Bytes):
		return &ConflictError{Path: path}
	}
	for _, file := range files {
		path := filepath.Join(directory, filepath.FromSlash(file.Path))
		found, err := DigestFile(ctx, path, file.Digest.Size)
		var tooLarge *TooLargeError
		switch {
		case errors.As(err, &tooLarge), errors.Is(err, fs.ErrNotExist):
			return &ConflictError{Path: path}
		case err != nil:
			return err
		case found != file.Digest:
			return &ConflictError{Path: path}
		}
	}
	return nil
}
