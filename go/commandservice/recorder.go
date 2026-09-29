package commandservice

// Bounded edit snapshots shared by a runner and its native command services
// (docs/execution/edit-tracking.md). One OS lock covers capture, mutation and
// publication. It is never held while waiting for command input or while
// running another command.

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"log/slog"
	"os"
	"slices"
	"strings"
	"syscall"
	"time"
	"unicode/utf8"

	"github.com/wspl/demi/go/artifact"
	"github.com/wspl/demi/go/internal/filelock"
)

// journalName is the file of a job's edit directory that holds its journal.
const journalName = "journal.json"

// A Recorder records the files a command edits into a job's edit directory.
// Handles on one job, in this process or another, share its journal.
type Recorder struct {
	context EditContext
}

// NewRecorder returns the recorder of the job that context names, and makes
// its edit directory.
func NewRecorder(context EditContext) (*Recorder, error) {
	if err := check(context); err != nil {
		return nil, err
	}
	if err := os.MkdirAll(context.Directory, 0o777); err != nil {
		return nil, err
	}
	return &Recorder{context: context}, nil
}

// Context returns where the recorder records.
func (r *Recorder) Context() EditContext {
	return r.context
}

// Begin takes the job's lock and starts a recording. A failure to record must
// not prevent the caller's filesystem operation, so it is reported as a
// diagnostic and Begin returns nil, which every method of a recording accepts.
func (r *Recorder) Begin() *Recording {
	recording, err := r.locked()
	if err != nil {
		diagnose(err)
		return nil
	}
	return recording
}

// Record runs operation, which changes the file at path, and records what it
// changed, even when it fails after writing. A path that is not a file, a pipe
// for one, is left alone: opening it can block until another command opens its
// other end.
func (r *Recorder) Record(path string, operation func() error) error {
	if info, err := os.Stat(path); err == nil && !info.Mode().IsRegular() {
		return operation()
	}
	recording := r.Begin()
	defer recording.Close()
	recording.Track(path)
	return operation()
}

// Report returns the job's journal, as the writers left it. It is called after
// all of them have stopped; contents come only from snapshots.
func (r *Recorder) Report() (EditJournal, error) {
	recording, err := r.locked()
	if err != nil {
		return EditJournal{}, err
	}
	defer recording.Close()
	recording.journal.Files = slices.DeleteFunc(recording.journal.Files, func(file EditFile) bool {
		if len(file.Edits) == 0 {
			return true
		}
		info, err := os.Stat(file.Path)
		if err != nil {
			// A file that cannot be looked at is kept: only one that is gone is
			// dropped.
			return absent(err)
		}
		return !info.Mode().IsRegular()
	})
	return recording.journal, nil
}

// locked takes the job's lock, and reads its journal.
func (r *Recorder) locked() (*Recording, error) {
	// Out of open files, recording waits for one rather than leave an edit out
	// (docs/execution/runner.md § Load).
	lock, err := RetryBlocking(func() (*os.File, error) {
		return os.OpenFile(r.context.Lock, os.O_RDWR|os.O_CREATE, 0o666)
	})
	if err != nil {
		return nil, err
	}
	if err := filelock.Lock(lock); err != nil {
		lock.Close()
		return nil, err
	}
	journal, err := r.readJournal()
	if err != nil {
		lock.Close()
		return nil, err
	}
	return &Recording{lock: lock, directory: r.context.Directory, journal: journal}, nil
}

// readJournal reads the journal of the job, which a job that recorded nothing
// yet does not have. A journal is decoded and checked as it enters, and its
// snapshots must be in the job's directory.
func (r *Recorder) readJournal() (EditJournal, error) {
	path, err := ResolvePath(r.context.Directory, journalName)
	if err != nil {
		return EditJournal{}, err
	}
	data, err := RetryBlocking(func() ([]byte, error) { return os.ReadFile(path) })
	if errors.Is(err, fs.ErrNotExist) {
		return EditJournal{}, nil
	}
	if err != nil {
		return EditJournal{}, err
	}
	journal, err := decode[EditJournal](data)
	if err != nil {
		return EditJournal{}, err
	}
	directory := PathKey(r.context.Directory)
	for _, file := range journal.Files {
		for _, edit := range file.Edits {
			if edit.Original != nil && edit.Modified == nil {
				return EditJournal{}, errors.New("original snapshot has no modified side")
			}
			for _, snapshot := range []*string{edit.Original, edit.Modified} {
				if snapshot != nil && !isIn(directory, *snapshot) {
					return EditJournal{}, errors.New("snapshot is outside its job directory")
				}
			}
		}
	}
	return journal, nil
}

// isIn reports whether path is a direct child of the directory whose [PathKey]
// is directory.
func isIn(directory, path string) bool {
	parent, ok := ParentPath(path)
	return ok && PathKey(parent) == directory
}

// A Recording is one holder of the job's lock: the files it tracks are
// compared with what they held when they were tracked once it is closed, and
// the changes are published to the journal. Its methods accept a nil Recording,
// which records nothing: a recording that could not begin.
type Recording struct {
	lock      *os.File
	directory string
	journal   EditJournal
	before    []snapshot
}

// A snapshot is what a tracked file held when it was tracked.
type snapshot struct {
	path     string
	contents contents
}

// Track notes what the file at path holds now. Track before touching the
// destination, including a backup rename.
func (r *Recording) Track(path string) {
	if r == nil {
		return
	}
	path = normalizedAbsolute(path)
	if slices.ContainsFunc(r.before, func(s snapshot) bool { return s.path == path }) {
		return
	}
	if len(r.before) >= EditJobFiles {
		r.journal.FilesTruncated = true
		return
	}
	var memory uint64
	for _, s := range r.before {
		memory += s.contents.size()
	}
	var found contents
	if memory+r.journal.BytesCopied >= EditJobBytes {
		found = statContents(path)
	} else {
		found = readContents(path)
	}
	if found.kind != notFile {
		r.before = append(r.before, snapshot{path, found})
	}
}

// Restored says that a transactional writer has restored the file at path to
// its exact prior bytes, so there is nothing to record of it.
func (r *Recording) Restored(path string) {
	if r == nil {
		return
	}
	path = normalizedAbsolute(path)
	r.before = slices.DeleteFunc(r.before, func(s snapshot) bool { return s.path == path })
}

// Close compares each tracked file with what it held, publishes the changes to
// the journal, and releases the job's lock. A change that cannot be recorded is
// a diagnostic: it leaves the file's record as metadata without contents.
func (r *Recording) Close() {
	if r == nil {
		return
	}
	defer r.lock.Close()
	before := r.before
	r.before = nil
	if len(before) == 0 {
		return
	}
	for _, s := range before {
		after := readContents(s.path)
		if err := r.publish(s.path, s.contents, after); err != nil {
			diagnose(err)
			if index := r.indexOf(s.path); index >= 0 {
				r.unavailable(index)
			}
		}
	}
	data, err := encode(r.journal)
	if err == nil {
		var path string
		path, err = ResolvePath(r.directory, journalName)
		if err == nil {
			err = publishFile(path, data)
		}
	}
	if err != nil {
		diagnose(err)
	}
}

// indexOf returns the index of the journal's file at path, or -1.
func (r *Recording) indexOf(path string) int {
	key := PathKey(path)
	return slices.IndexFunc(r.journal.Files, func(file EditFile) bool { return PathKey(file.Path) == key })
}

// publish records that the file at path went from before to after.
func (r *Recording) publish(path string, before, after contents) error {
	if before.same(after) || after.kind == notFile {
		return nil
	}
	name := strings.ToValidUTF8(path, "�")
	index := slices.IndexFunc(r.journal.Files, func(file EditFile) bool { return file.Path == name })
	if index < 0 {
		switch {
		case after.kind == missing:
			// Deletions have no independent entry. A containing transaction can
			// still restore the file before this recording captures its final
			// side.
			return nil
		case len(r.journal.Files) >= EditJobFiles:
			r.journal.FilesTruncated = true
			return nil
		}
		kind := EditModified
		if before.kind == missing {
			kind = EditAdded
		}
		r.journal.Files = append(r.journal.Files, EditFile{Path: name, Kind: kind})
		index = len(r.journal.Files) - 1
	}
	file := &r.journal.Files[index]
	var previous *EditCopies
	if len(file.Edits) > 0 {
		previous = &file.Edits[len(file.Edits)-1]
	}
	if previous != nil && previous.Modified == nil {
		// Once continuity is unknown, keep metadata instead of a stale diff.
		return nil
	}
	merge := previous != nil && before.same(readContents(*previous.Modified))
	original := before
	if merge {
		original = contents{kind: missing}
		if previous.Original != nil {
			original = readContents(*previous.Original)
		}
	}
	if merge && original.same(after) {
		file.Edits = file.Edits[:len(file.Edits)-1]
		return nil
	}
	if after.kind == missing {
		// The final report omits absent paths. Re-creation cannot reuse the
		// previous after side, so keep no misleading continuous segment.
		file.Edits = nil
		return nil
	}
	copyBytes := after.size()
	if !merge {
		copyBytes += original.size()
	}
	if !original.textOrMissing() || !after.textOrMissing() || r.journal.BytesCopied+copyBytes > EditJobBytes {
		r.unavailable(index)
		return nil
	}
	if !merge && r.journal.NextSegment >= EditJobSegments {
		r.journal.FilesTruncated = true
		r.unavailable(index)
		return nil
	}
	var copies EditCopies
	if merge {
		copies = *previous
	} else {
		segment := r.journal.NextSegment
		r.journal.NextSegment++
		if original.kind != missing {
			path, err := ResolvePath(r.directory, fmt.Sprintf("%d.original", segment))
			if err != nil {
				return err
			}
			copies.Original = &path
		}
		path, err := ResolvePath(r.directory, fmt.Sprintf("%d.modified", segment))
		if err != nil {
			return err
		}
		copies.Modified = &path
	}
	if !merge && copies.Original != nil && original.kind == held {
		if err := publishFile(*copies.Original, original.data); err != nil {
			return err
		}
		r.journal.BytesCopied += uint64(len(original.data))
	}
	if copies.Modified != nil && after.kind == held {
		if err := publishFile(*copies.Modified, after.data); err != nil {
			return err
		}
		r.journal.BytesCopied += uint64(len(after.data))
	}
	if merge {
		file.Edits[len(file.Edits)-1] = copies
	} else {
		file.Edits = append(file.Edits, copies)
	}
	return nil
}

// unavailable leaves the file's record as metadata without contents.
func (r *Recording) unavailable(index int) {
	r.journal.Files[index].Edits = []EditCopies{{}}
}

// The kinds of contents a file can be found with.
type contentsKind int

const (
	// missing is a file that does not exist.
	missing contentsKind = iota
	// held is a file whose bytes were read.
	held
	// unavailable is a file whose bytes were not read: it is too large, or
	// cannot be.
	unavailable
	// notFile is a path that is not a regular file.
	notFile
)

// contents is what a path held when it was looked at.
type contents struct {
	kind contentsKind
	// data are the bytes of held contents.
	data []byte
	// stamp tells a held or unavailable file from another by its size, its
	// modification time and its creation time, when they are known.
	stamp *fileStamp
}

type fileStamp struct {
	length   int64
	modified time.Time
	// created is when the file was created, where the system and the file system
	// tell; two stamps are equal only if both know it and agree, or neither does.
	created    time.Time
	hasCreated bool
}

func stampOf(path string, info fs.FileInfo) *fileStamp {
	stamp := &fileStamp{length: info.Size(), modified: info.ModTime()}
	stamp.created, stamp.hasCreated = birthTime(path, info)
	return stamp
}

func (s *fileStamp) equal(other *fileStamp) bool {
	if s.length != other.length || !s.modified.Equal(other.modified) || s.hasCreated != other.hasCreated {
		return false
	}
	return !s.hasCreated || s.created.Equal(other.created)
}

// absent reports whether a lookup failed because nothing is at the path: a path
// below a file names nothing either (docs/execution/edit-tracking.md
// § Recording actual writes).
func absent(err error) bool {
	return errors.Is(err, fs.ErrNotExist) || errors.Is(err, syscall.ENOTDIR)
}

// statContents looks at a path without reading it.
func statContents(path string) contents {
	info, err := os.Stat(path)
	switch {
	case absent(err):
		return contents{kind: missing}
	case err != nil:
		return contents{kind: unavailable}
	case !info.Mode().IsRegular():
		return contents{kind: notFile}
	}
	return contents{kind: unavailable, stamp: stampOf(path, info)}
}

// readContents reads a path: what it holds up to [EditFileBytes] bytes.
func readContents(path string) contents {
	info, err := os.Stat(path)
	switch {
	case err == nil && !info.Mode().IsRegular():
		return contents{kind: notFile}
	case err == nil && info.Size() > EditFileBytes:
		return contents{kind: unavailable, stamp: stampOf(path, info)}
	case absent(err):
		return contents{kind: missing}
	case err != nil:
		return contents{kind: unavailable}
	}
	stamp := stampOf(path, info)
	data, err := RetryBlocking(func() ([]byte, error) { return readLimited(path) })
	if err != nil || len(data) > EditFileBytes {
		return contents{kind: unavailable, stamp: stamp}
	}
	return contents{kind: held, data: data, stamp: stamp}
}

// readLimited reads the file at path, and one byte more than [EditFileBytes] of
// a longer one.
func readLimited(path string) ([]byte, error) {
	file, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	return io.ReadAll(io.LimitReader(file, EditFileBytes+1))
}

// same reports whether two contents are of one file state.
func (c contents) same(other contents) bool {
	switch {
	case c.kind == missing && other.kind == missing:
		return true
	case c.kind == held && other.kind == held:
		return bytes.Equal(c.data, other.data)
	}
	// Failed opens and other no-ops must not invent edits when contents exceed
	// the snapshot budget. Unknown metadata proves nothing.
	known := func(x contents) bool { return x.stamp != nil && (x.kind == held || x.kind == unavailable) }
	if known(c) && known(other) && (c.kind == unavailable || other.kind == unavailable) {
		return c.stamp.equal(other.stamp)
	}
	return false
}

// textOrMissing reports whether contents are text, which the record copies, or
// nothing.
func (c contents) textOrMissing() bool {
	switch c.kind {
	case missing:
		return true
	case held:
		return IsText(c.data)
	}
	return false
}

// size returns the bytes that copying the contents costs.
func (c contents) size() uint64 {
	if c.kind == held {
		return uint64(len(c.data))
	}
	return 0
}

// publishFile publishes a snapshot or the journal at path through artifact's
// atomic publication, readable by its owner alone as the files it copies may not
// be. Nothing needs it to survive a crash, so it is not synced.
func publishFile(path string, data []byte) error {
	publication := artifact.Publication{Mode: artifact.Replace, Permissions: artifact.Private}
	_, err := RetryBlocking(func() (struct{}, error) {
		return struct{}{}, artifact.PublishBytes(path, data, publication)
	})
	return err
}

// IsText reports whether data is text, which edit tracking and line counts read:
// UTF-8 without a NUL byte. Binary and non-UTF-8 content are treated alike
// (docs/execution/edit-tracking.md § Scope).
func IsText(data []byte) bool {
	return bytes.IndexByte(data, 0) < 0 && utf8.Valid(data)
}

// diagnose reports that recording an edit failed. Inside a command service it
// reaches standard error, which the runner drains into the Host's log
// (docs/execution/edit-tracking.md).
func diagnose(err error) {
	slog.Warn("edit recording failed", "error", err)
}
