package builtincommands

import (
	"cmp"
	"context"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"log/slog"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/go/artifact"
	"github.com/wspl/demi/go/builtinproto"
	"github.com/wspl/demi/go/commandservice"
)

// readBytes is how much of a file one read sends on.
const readBytes = 64 * 1024

// read streams the file to stdout. A cancelled invocation stops it, even when
// it waits on a file that does not end (a pipe).
func (h *Handler) read(call *commandservice.Call, args builtinproto.ReadArgs) error {
	ctx := call.Context()
	path, err := commandservice.ResolvePath(call.Invocation.Cwd, args.Path)
	if err != nil {
		return err
	}
	file, err := os.Open(path)
	if err != nil {
		return system(err)
	}
	defer file.Close()
	// Closing the file wakes a read that waits; the read then reports that it was
	// closed, and the cancellation is what is answered.
	stop := context.AfterFunc(ctx, func() { _ = file.Close() })
	defer stop()
	buffer := make([]byte, readBytes)
	for {
		if ctx.Err() != nil {
			return errCancelled
		}
		count, err := file.Read(buffer)
		if count > 0 {
			if _, err := call.Stdout.Write(buffer[:count]); err != nil {
				return err
			}
		}
		switch {
		case err == io.EOF:
			return nil
		case err != nil && ctx.Err() != nil:
			return errCancelled
		case err != nil:
			return system(err)
		}
	}
}

// create makes a new file and its directories; an existing file is left as it is.
func (h *Handler) create(call *commandservice.Call, args builtinproto.CreateArgs) error {
	ctx := call.Context()
	cwd := call.Invocation.Cwd
	return h.mutate(call, func(recording *commandservice.Recording) (string, error) {
		path, err := commandservice.ResolvePath(cwd, args.Path)
		if err != nil {
			return "", err
		}
		recording.Track(path)
		if err := checkCancelled(ctx); err != nil {
			return "", err
		}
		if err := atomicWrite(path, []byte(args.Content), true); err != nil {
			return "", err
		}
		return "Created " + args.Path + "\n", nil
	})
}

// edit replaces one occurrence of exact text in an existing file: the only one,
// the one that is asked for by its number, or the one nearest to a line.
func (h *Handler) edit(call *commandservice.Call, args builtinproto.EditArgs) error {
	ctx := call.Context()
	cwd := call.Invocation.Cwd
	return h.mutate(call, func(recording *commandservice.Recording) (string, error) {
		path, err := commandservice.ResolvePath(cwd, args.Path)
		if err != nil {
			return "", err
		}
		content, err := readText(path)
		if err != nil {
			return "", err
		}
		matches := matchIndices(content, args.Old)
		if err := checkCancelled(ctx); err != nil {
			return "", err
		}
		index, err := chooseMatch(content, matches, args)
		if err != nil {
			return "", err
		}
		if args.Old == args.New {
			return "Edited " + args.Path + "\n", nil
		}
		updated := content[:index] + args.New + content[index+len(args.Old):]
		if err := checkCancelled(ctx); err != nil {
			return "", err
		}
		recording.Track(path)
		if err := atomicWrite(path, []byte(updated), false); err != nil {
			return "", err
		}
		return "Edited " + args.Path + "\n", nil
	})
}

// chooseMatch returns the byte index of the occurrence an edit replaces.
func chooseMatch(content string, matches []int, args builtinproto.EditArgs) (int, error) {
	switch {
	case args.Occurrence != nil:
		occurrence := uint64(*args.Occurrence)
		if occurrence-1 >= uint64(len(matches)) {
			return 0, fail("Occurrence %d is out of range", occurrence)
		}
		return matches[occurrence-1], nil
	case args.Context != nil:
		return nearest(content, matches, uint64(*args.Context))
	}
	switch len(matches) {
	case 1:
		return matches[0], nil
	case 0:
		return 0, fail("No match found in %s", args.Path)
	}
	return 0, fail("Multiple matches in %s; specify --occurrence or --context", args.Path)
}

// nearest returns the occurrence nearest to the line target, and refuses two
// that are as near.
func nearest(content string, matches []int, target uint64) (int, error) {
	type ranked struct {
		distance uint64
		index    int
	}
	candidates := make([]ranked, len(matches))
	for i, index := range matches {
		line := uint64(lineOf(content, index))
		candidates[i] = ranked{distance: max(line, target) - min(line, target), index: index}
	}
	slices.SortFunc(candidates, func(a, b ranked) int {
		return cmp.Or(cmp.Compare(a.distance, b.distance), cmp.Compare(a.index, b.index))
	})
	if len(candidates) == 0 {
		return 0, fail("No match found")
	}
	if len(candidates) > 1 && candidates[1].distance == candidates[0].distance {
		listed := make([]string, len(matches))
		for occurrence, index := range matches {
			listed[occurrence] = fmt.Sprintf("occurrence %d at line %d", occurrence+1, lineOf(content, index))
		}
		return 0, fail("Context line %d is ambiguous: %s", target, strings.Join(listed, "; "))
	}
	return candidates[0].index, nil
}

// matchIndices returns the byte index of each non-overlapping occurrence of old
// in content, from the left.
func matchIndices(content, old string) []int {
	var indices []int
	for offset := 0; ; {
		found := strings.Index(content[offset:], old)
		if found < 0 {
			return indices
		}
		indices = append(indices, offset+found)
		offset += found + len(old)
	}
}

// lineOf returns the 1-based line of the byte at index.
func lineOf(content string, index int) int {
	return strings.Count(content[:index], "\n") + 1
}

// readText reads a file that holds text.
func readText(path string) (string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return "", system(err)
	}
	if !utf8.Valid(data) {
		return "", errNotUTF8
	}
	return string(data), nil
}

// checkCancelled returns the failure of a cancelled invocation.
func checkCancelled(ctx context.Context) error {
	if ctx.Err() != nil {
		return errCancelled
	}
	return nil
}

// mutate runs a mutation of files while it holds the gate that lets one run at a
// time, and sends its message on. The gate covers planning, writes and rollback,
// and the edits are recorded before the message is sent.
func (h *Handler) mutate(call *commandservice.Call, mutation func(*commandservice.Recording) (string, error)) error {
	message, err := h.locked(call, mutation)
	if err != nil {
		return err
	}
	_, err = io.WriteString(call.Stdout, message)
	return err
}

// locked runs mutation while it holds the gate, recording the files it edits
// when the invocation asks for edits to be recorded.
func (h *Handler) locked(call *commandservice.Call, mutation func(*commandservice.Recording) (string, error)) (string, error) {
	ctx := call.Context()
	permit, err := h.mutations.Acquire(ctx)
	if err != nil {
		return "", errCancelled
	}
	defer permit.Release()
	recording := beginRecording(call.Invocation.Edits)
	defer recording.Close()
	if err := checkCancelled(ctx); err != nil {
		return "", err
	}
	return mutation(recording)
}

// beginRecording starts recording into the job's edit directory. A failure to
// record must not prevent the file operation, so it is a diagnostic and the
// operation goes on without a recording, which is nil.
func beginRecording(edits *commandservice.EditContext) *commandservice.Recording {
	if edits == nil {
		return nil
	}
	recorder, err := commandservice.NewRecorder(*edits)
	if err != nil {
		slog.Warn("edit recording failed", "error", err)
		return nil
	}
	return recorder.Begin()
}

// atomicWrite replaces the file at path with data, or creates it when create,
// in one step a reader never sees half done, and makes its directories. An edit
// of a symbolic link writes the file it points at and keeps that file's
// permissions.
func atomicWrite(path string, data []byte, create bool) error {
	destination := path
	if !create {
		if info, err := os.Lstat(path); err == nil && info.Mode()&fs.ModeSymlink != 0 {
			resolved, err := canonicalize(path)
			if err != nil {
				return system(err)
			}
			destination = resolved
		}
	}
	parent, ok := commandservice.ParentPath(destination)
	if !ok {
		return errNoParent
	}
	if err := createDirAll(parent); err != nil {
		return system(err)
	}
	publication := artifact.Publication{Mode: artifact.Replace, Permissions: artifact.Keep, Durable: true}
	if create {
		publication = artifact.Publication{Mode: artifact.CreateNew, Permissions: artifact.DefaultPermissions, Durable: true}
	}
	return system(artifact.PublishBytes(destination, data, publication))
}

// canonicalize returns the absolute path of an existing file with every link
// resolved.
func canonicalize(path string) (string, error) {
	resolved, err := filepath.EvalSymlinks(path)
	if err != nil {
		return "", err
	}
	return filepath.Abs(resolved)
}

// createDirAll makes a directory and the ones above it that are missing, as
// Rust's create_dir_all does: the directory is made first, and only when its
// parent is missing are the parents made, so a failure is the system's own for
// the directory asked for. os.MkdirAll is not used: for a path that exists as a
// file it answers "not a directory" where the Rust answers "file exists", and
// the agent reads the message.
func createDirAll(path string) error {
	if path == "" {
		return nil
	}
	err := os.Mkdir(path, 0o777)
	switch {
	case err == nil:
		return nil
	case errors.Is(err, fs.ErrNotExist):
	case isDirectory(path):
		return nil
	default:
		return err
	}
	parent, ok := commandservice.ParentPath(path)
	if !ok {
		return errors.New("failed to create whole tree")
	}
	if err := createDirAll(parent); err != nil {
		return err
	}
	err = os.Mkdir(path, 0o777)
	if err == nil || isDirectory(path) {
		return nil
	}
	return err
}

func isDirectory(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
}
