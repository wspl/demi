package cmdsdk

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"syscall"
	"time"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/commandwire"
)

// Recorder records bounded snapshots in a job journal shared across processes.
type Recorder struct{ context commandwire.EditContext }

// NewRecorder validates the job paths and creates its snapshot directory.
func NewRecorder(ctx context.Context, c commandwire.EditContext) (*Recorder, error) {
	if err := c.Validate(); err != nil {
		return nil, err
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	err := os.MkdirAll(c.Directory, 0o777)
	if err != nil {
		return nil, err
	}
	return &Recorder{context: c}, nil
}

// Context returns the paths to pass to other writers of this job.
func (r *Recorder) Context() commandwire.EditContext { return r.context }

// Begin takes the job's OS lock. Failure to record must not prevent the filesystem operation.
// Defer Close on a non-nil result so partial writes are captured on failure as well.
func (r *Recorder) Begin(ctx context.Context) *Recording {
	recording, err := r.lockRecording(ctx)
	if err != nil {
		slog.Warn("edit recording failed", "error", err)
		return nil
	}
	return recording
}

// Record captures a single operation's edits, including partial changes on error.
// Nonregular files bypass recording so opening a pipe never holds the job lock.
func (r *Recorder) Record(ctx context.Context, path string, operation func() error) error {
	if info, err := os.Stat(path); err == nil && !info.Mode().IsRegular() {
		return operation()
	}
	recording := r.Begin(ctx)
	if recording != nil {
		defer recording.Close(ctx)
		recording.Track(ctx, path)
	}
	return operation()
}

// Report reads snapshots after writers have stopped; absent and nonregular paths are omitted.
func (r *Recorder) Report(ctx context.Context) (commandwire.EditJournal, error) {
	recording, err := r.lockRecording(ctx)
	if err != nil {
		return commandwire.EditJournal{}, err
	}
	defer recording.Close(ctx)
	recording.journal.Files = slices.DeleteFunc(recording.journal.Files, func(f commandwire.EditFile) bool {
		if len(f.Edits) == 0 {
			return true
		}
		info, err := os.Stat(f.Path)
		if err != nil {
			return absent(err)
		}
		return !info.Mode().IsRegular()
	})
	return recording.journal, nil
}

// lockRecording acquires the job lock and validates its stored journal.
func (r *Recorder) lockRecording(ctx context.Context) (*Recording, error) {
	lock, err := Retry(
		ctx,
		func() (*artifacts.InstallLock, error) { return artifacts.AcquireInstallLock(ctx, r.context.Lock) },
	)
	if err != nil {
		return nil, err
	}
	journal := commandwire.EditJournal{Files: []commandwire.EditFile{}}
	b, err := Retry(
		ctx,
		func() ([]byte, error) { return os.ReadFile(filepath.Join(r.context.Directory, "journal.json")) },
	)
	if err == nil {
		journal, err = commandwire.DecodeEditJournal(b)
		if err == nil {
			err = checkJournalSnapshots(journal, r.context.Directory)
		}
	} else if errors.Is(err, os.ErrNotExist) {
		err = nil
	}
	if err != nil {
		return nil, errors.Join(err, lock.Close())
	}
	return &Recording{lock: lock, directory: r.context.Directory, journal: journal}, nil
}

// Recording holds the job lock across capture, mutation and publication.
// Never hold it while waiting for command input or another command.
type Recording struct {
	lock      *artifacts.InstallLock
	directory string
	journal   commandwire.EditJournal
	before    []tracked
}
type tracked struct {
	path     string
	contents contents
}

// Track captures a destination before any change, including backup renames.
func (r *Recording) Track(ctx context.Context, path string) {
	path = normalize(path)
	for _, p := range r.before {
		if p.path == path {
			return
		}
	}
	if len(r.before) >= commandwire.EditJobFiles {
		r.journal.FilesTruncated = true
		return
	}
	memory := r.journal.BytesCopied
	for _, p := range r.before {
		memory += uint64(len(p.contents.data))
	}
	before := readContents(ctx, path, memory >= commandwire.EditJobBytes)
	if before.kind != notFile {
		r.before = append(r.before, tracked{path, before})
	}
}

// Restored forgets a destination a transactional writer restored to its exact prior bytes.
func (r *Recording) Restored(path string) {
	path = normalize(path)
	r.before = slices.DeleteFunc(r.before, func(p tracked) bool { return p.path == path })
}

// Close captures final contents and publishes the journal, then releases the lock.
// Recording failures are diagnostic only and never replace the operation's result.
func (r *Recording) Close(ctx context.Context) {
	if r.lock == nil {
		return
	}
	defer func() {
		if err := r.lock.Close(); err != nil {
			slog.Warn("edit recording failed", "error", err)
		}
		r.lock = nil
	}()
	if len(r.before) == 0 {
		return
	}
	// Publication must capture partial mutations even after invocation cancellation.
	ctx = context.WithoutCancel(ctx)
	for _, p := range r.before {
		if err := r.publish(ctx, p.path, p.contents, readContents(ctx, p.path, false)); err != nil {
			slog.Warn("edit recording failed", "error", err)
			for index, f := range r.journal.Files {
				if f.Path == p.path {
					r.unavailable(index)
				}
			}
		}
	}
	b, err := r.journal.MarshalJSON()
	if err == nil {
		err = publishSnapshot(ctx, filepath.Join(r.directory, "journal.json"), b)
	}
	if err != nil {
		slog.Warn("edit recording failed", "error", err)
	}
}

func (r *Recording) unavailable(index int) {
	r.journal.Files[index].Edits = []commandwire.EditCopies{{}}
}

// publish updates a changed file's snapshots and journal entry.
func (r *Recording) publish(ctx context.Context, path string, before, after contents) error {
	if before.same(after) || after.kind == notFile {
		return nil
	}
	index := r.editFile(path, before, after)
	if index < 0 {
		return nil
	}
	edits := r.journal.Files[index].Edits
	var previous commandwire.EditCopies
	merge := false
	if len(edits) > 0 {
		previous = edits[len(edits)-1]
		if previous.Modified == nil {
			return nil
		}
		merge = before.same(readContents(ctx, *previous.Modified, false))
	}
	original := before
	if merge {
		original = contents{kind: missing}
		if previous.Original != nil {
			original = readContents(ctx, *previous.Original, false)
		}
	}
	if merge && original.same(after) {
		r.journal.Files[index].Edits = edits[:len(edits)-1]
		return nil
	}
	if after.kind == missing {
		r.journal.Files[index].Edits = []commandwire.EditCopies{}
		return nil
	}
	copied := uint64(len(after.data))
	if !merge {
		copied += uint64(len(original.data))
	}
	if !original.text() || !after.text() || r.journal.BytesCopied+copied > commandwire.EditJobBytes {
		r.unavailable(index)
		return nil
	}
	if !merge && r.journal.NextSegment >= commandwire.EditJobSegments {
		r.journal.FilesTruncated = true
		r.unavailable(index)
		return nil
	}
	copies, err := r.publishCopies(ctx, previous, original, after, merge)
	if err != nil {
		return err
	}
	if merge {
		r.journal.Files[index].Edits[len(edits)-1] = copies
	} else {
		r.journal.Files[index].Edits = append(edits, copies)
	}
	return nil
}

type contentKind uint8

const (
	missing contentKind = iota
	contentBytes
	unavailable
	notFile
)

type contents struct {
	kind    contentKind
	data    []byte
	info    os.FileInfo
	created time.Time
}

func absent(err error) bool { return errors.Is(err, os.ErrNotExist) || errors.Is(err, syscall.ENOTDIR) }

func readContents(ctx context.Context, path string, metadataOnly bool) contents {
	info, err := os.Stat(path)
	if absent(err) {
		return contents{kind: missing}
	}
	if err != nil {
		return contents{kind: unavailable}
	}
	if !info.Mode().IsRegular() {
		return contents{kind: notFile}
	}
	result := contents{kind: unavailable, info: info, created: creationStamp(path, info)}
	if metadataOnly || info.Size() > commandwire.EditFileBytes {
		return result
	}
	file, err := Retry(ctx, func() (*os.File, error) { return os.Open(path) })
	if err != nil {
		return result
	}
	b, err := io.ReadAll(io.LimitReader(file, commandwire.EditFileBytes+1))
	err = errors.Join(err, file.Close())
	if err == nil && len(b) <= commandwire.EditFileBytes {
		result.kind = contentBytes
		result.data = b
	}
	return result
}

func (c contents) same(other contents) bool {
	if c.kind == missing && other.kind == missing {
		return true
	}
	if c.kind == contentBytes && other.kind == contentBytes {
		return bytes.Equal(c.data, other.data)
	}
	if (c.kind == unavailable && (other.kind == unavailable || other.kind == contentBytes)) ||
		(other.kind == unavailable && c.kind == contentBytes) {
		return c.info != nil && other.info != nil && c.info.Size() == other.info.Size() &&
			c.info.ModTime().Equal(other.info.ModTime()) &&
			c.created.Equal(other.created)
	}
	return false
}

func (c contents) text() bool {
	return c.kind == missing || (c.kind == contentBytes && commandwire.IsText(c.data))
}

func publishSnapshot(ctx context.Context, path string, b []byte) error {
	_, err := Retry(ctx, func() (struct{}, error) {
		return struct{}{}, artifacts.PublishBytes(
			ctx,
			path,
			b,
			artifacts.Publication{Mode: artifacts.Replace, Permissions: artifacts.Private},
		)
	})
	return err
}

// normalize makes recorder paths absolute, removing dots and redundant separators
// while retaining '..', whose meaning can depend on a preceding symlink.
func normalize(path string) string {
	if !filepath.IsAbs(path) {
		cwd, err := os.Getwd()
		if err != nil {
			return path
		}
		absolute, err := Resolve(cwd, path)
		if err != nil {
			return path
		}
		path = absolute
	}
	volume := filepath.VolumeName(path)
	parts := strings.FieldsFunc(
		path[len(volume):],
		func(c rune) bool { return c == rune(os.PathSeparator) || (os.PathSeparator == '\\' && c == '/') },
	)
	parts = slices.DeleteFunc(parts, func(part string) bool { return part == "." })
	return volume + string(os.PathSeparator) + strings.Join(parts, string(os.PathSeparator))
}

// checkJournalSnapshots checks that journal snapshots belong to this job.
func checkJournalSnapshots(journal commandwire.EditJournal, directory string) error {
	var err error
	for _, file := range journal.Files {
		for _, edit := range file.Edits {
			if edit.Original != nil && edit.Modified == nil {
				err = errors.New("original snapshot has no modified side")
				break
			}
			for _, path := range []*string{edit.Original, edit.Modified} {
				if path != nil && filepath.Dir(*path) != directory {
					err = errors.New("snapshot is outside its job directory")
					break
				}
			}
		}
	}
	return err
}

// editFile locates or registers a changed file within the job limit.
func (r *Recording) editFile(path string, before, after contents) int {
	index := slices.IndexFunc(r.journal.Files, func(f commandwire.EditFile) bool { return f.Path == path })
	if index < 0 {
		if after.kind == missing {
			return -1
		}
		if len(r.journal.Files) >= commandwire.EditJobFiles {
			r.journal.FilesTruncated = true
			return -1
		}
		kind := commandwire.EditModified
		if before.kind == missing {
			kind = commandwire.EditAdded
		}
		r.journal.Files = append(
			r.journal.Files,
			commandwire.EditFile{Path: path, Kind: kind, Edits: []commandwire.EditCopies{}},
		)
		index = len(r.journal.Files) - 1
	}
	return index
}

// publishCopies writes the snapshot sides and accounts for copied bytes.
func (r *Recording) publishCopies(
	ctx context.Context,
	previous commandwire.EditCopies,
	original, after contents,
	merge bool,
) (commandwire.EditCopies, error) {
	copies := previous
	if !merge {
		segment := r.journal.NextSegment
		r.journal.NextSegment++
		modified := filepath.Join(r.directory, fmt.Sprintf("%d.modified", segment))
		copies = commandwire.EditCopies{Modified: &modified}
		if original.kind != missing {
			p := filepath.Join(r.directory, fmt.Sprintf("%d.original", segment))
			copies.Original = &p
		}
	}
	if !merge && copies.Original != nil && original.kind == contentBytes {
		if err := publishSnapshot(ctx, *copies.Original, original.data); err != nil {
			return commandwire.EditCopies{}, err
		}
		r.journal.BytesCopied += uint64(len(original.data))
	}
	if copies.Modified != nil && after.kind == contentBytes {
		if err := publishSnapshot(ctx, *copies.Modified, after.data); err != nil {
			return commandwire.EditCopies{}, err
		}
		r.journal.BytesCopied += uint64(len(after.data))
	}
	return copies, nil
}
