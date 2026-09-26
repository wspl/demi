package shell

// This file is the job's filesystem access: toolctx.Files for utilities and
// the interpreter's open handler. It is the one place in internal/shell that
// touches the filesystem directly, which the forbidigo rule otherwise bans.

import (
	"context"
	"errors"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"syscall"
	"time"

	"mvdan.cc/sh/v3/interp"

	"github.com/wspl/demi/internal/toolctx"
)

// files is the filesystem as one command of a job sees it.
type files struct {
	job *Job
	// dir is the command's working directory.
	dir string
	// umask is the job's mask at the time of the command.
	umask fs.FileMode
	// record is false for the utilities whose writes are not edits:
	// copies (cp), moves (mv) and the empty file of mktemp.
	record bool
	// streams are what "/dev/stdin", "/dev/fd/3" and the like name.
	streams streams
}

var _ toolctx.Files = (*files)(nil)

// streams are a command's descriptors.
type streams struct {
	stdin       io.Reader
	stdout      io.Writer
	stderr      io.Writer
	descriptors map[int]interp.Descriptor
}

// descriptor returns the command's descriptor fd.
func (s streams) descriptor(fd int) (interp.Descriptor, bool) {
	switch fd {
	case 0:
		return interp.Descriptor{Reader: s.stdin}, s.stdin != nil
	case 1:
		return interp.Descriptor{Writer: s.stdout}, s.stdout != nil
	case 2:
		return interp.Descriptor{Writer: s.stderr}, s.stderr != nil
	}
	d, ok := s.descriptors[fd]
	return d, ok
}

// Abs implements toolctx.Files.
func (f *files) Abs(name string) string {
	if filepath.IsAbs(name) {
		return filepath.Clean(name)
	}
	return filepath.Join(f.dir, name)
}

// Open implements toolctx.Files.
func (f *files) Open(name string) (toolctx.File, error) {
	return f.OpenFile(name, os.O_RDONLY, 0)
}

// OpenFile implements toolctx.Files. A path such as "/dev/stdout" or
// "/dev/fd/3" opens the command's own descriptor. A new file gets perm
// reduced by the job's umask.
func (f *files) OpenFile(name string, flag int, perm fs.FileMode) (toolctx.File, error) {
	if fd, ok := interp.DescriptorPath(name); ok {
		d, ok := f.streams.descriptor(fd)
		if !ok {
			return nil, &fs.PathError{Op: "open", Path: name, Err: fs.ErrNotExist}
		}
		return &streamFile{name: name, d: d}, nil
	}
	path := f.Abs(name)
	var recorder EditRecorder
	if flag&(os.O_WRONLY|os.O_RDWR) != 0 && f.recording(path) {
		recorder = f.job.spec.Edits
	}
	var file *os.File
	var err error
	open := func() {
		file, err = f.openCreate(path, flag, perm)
	}
	if recorder != nil {
		recorder.Record(path, open)
	} else {
		open()
	}
	if err != nil {
		return nil, asGiven(err, name, "")
	}
	return f.track(&jobFile{file: file, name: name, path: path, recorder: recorder})
}

// track makes cancellation close file.
func (f *files) track(file *jobFile) (toolctx.File, error) {
	file.job = f.job
	if !f.job.track(file) {
		return nil, &fs.PathError{Op: "open", Path: file.name, Err: context.Canceled}
	}
	return file, nil
}

// recording reports whether a write to path is recorded: the job records
// edits, this command's writes are edits, and path is a regular file or
// does not exist.
func (f *files) recording(path string) bool {
	if !f.record || f.job.spec.Edits == nil {
		return false
	}
	info, err := os.Stat(path) //nolint:forbidigo // toolctx.Files implementation
	if errors.Is(err, fs.ErrNotExist) {
		return true
	}
	return err == nil && info.Mode().IsRegular()
}

// openCreate opens path, waiting for a descriptor when none is left. A file
// it creates gets exactly perm without the job's umask, whatever the
// process umask is.
func (f *files) openCreate(path string, flag int, perm fs.FileMode) (*os.File, error) {
	mode := perm &^ f.umask
	created := false
	if flag&os.O_CREATE != 0 {
		_, err := os.Stat(path) //nolint:forbidigo // toolctx.Files implementation
		created = errors.Is(err, fs.ErrNotExist)
	}
	file, err := waitForDescriptors(f.job.ctx, func() (*os.File, error) {
		return os.OpenFile(path, flag, mode) //nolint:forbidigo // toolctx.Files implementation
	})
	if err != nil || !created {
		return file, err
	}
	// The process umask may have removed more than the job's. If this fails,
	// the file keeps the mode the process umask left, which is no reason to
	// fail the open.
	file.Chmod(mode)
	return file, nil
}

// Stat implements toolctx.Files.
func (f *files) Stat(name string) (fs.FileInfo, error) {
	if fd, ok := interp.DescriptorPath(name); ok {
		return f.descriptorStat(name, fd)
	}
	info, err := os.Stat(f.Abs(name)) //nolint:forbidigo // toolctx.Files implementation
	return info, asGiven(err, name, "")
}

// Lstat implements toolctx.Files.
func (f *files) Lstat(name string) (fs.FileInfo, error) {
	if fd, ok := interp.DescriptorPath(name); ok {
		return f.descriptorStat(name, fd)
	}
	info, err := os.Lstat(f.Abs(name)) //nolint:forbidigo // toolctx.Files implementation
	return info, asGiven(err, name, "")
}

func (f *files) descriptorStat(name string, fd int) (fs.FileInfo, error) {
	d, ok := f.streams.descriptor(fd)
	if !ok {
		return nil, &fs.PathError{Op: "stat", Path: name, Err: fs.ErrNotExist}
	}
	return (&streamFile{name: name, d: d}).Stat()
}

// ReadDir implements toolctx.Files.
func (f *files) ReadDir(name string) ([]fs.DirEntry, error) {
	entries, err := os.ReadDir(f.Abs(name)) //nolint:forbidigo // toolctx.Files implementation
	return entries, asGiven(err, name, "")
}

// Readlink implements toolctx.Files.
func (f *files) Readlink(name string) (string, error) {
	target, err := os.Readlink(f.Abs(name)) //nolint:forbidigo // toolctx.Files implementation
	return target, asGiven(err, name, "")
}

// Mkdir implements toolctx.Files. The directory gets perm without the job's
// umask.
func (f *files) Mkdir(name string, perm fs.FileMode) error {
	return asGiven(f.mkdir(f.Abs(name), perm), name, "")
}

func (f *files) mkdir(path string, perm fs.FileMode) error {
	mode := perm &^ f.umask
	if err := os.Mkdir(path, mode); err != nil { //nolint:forbidigo // toolctx.Files implementation
		return err
	}
	// As for files, the process umask may have removed more than the job's.
	return os.Chmod(path, mode) //nolint:forbidigo // toolctx.Files implementation
}

// MkdirAll implements toolctx.Files like os.MkdirAll, with the job's umask.
func (f *files) MkdirAll(name string, perm fs.FileMode) error {
	return asGiven(f.mkdirAll(f.Abs(name), perm), name, "")
}

func (f *files) mkdirAll(path string, perm fs.FileMode) error {
	info, err := os.Stat(path) //nolint:forbidigo // toolctx.Files implementation
	if err == nil {
		if info.IsDir() {
			return nil
		}
		return &fs.PathError{Op: "mkdir", Path: path, Err: syscall.ENOTDIR}
	}
	if parent := filepath.Dir(path); parent != path {
		if err := f.mkdirAll(parent, perm); err != nil {
			return err
		}
	}
	err = f.mkdir(path, perm)
	if errors.Is(err, fs.ErrExist) {
		// Another command created it in between.
		info, statErr := os.Lstat(path) //nolint:forbidigo // toolctx.Files implementation
		if statErr == nil && info.IsDir() {
			return nil
		}
	}
	return err
}

// Remove implements toolctx.Files.
func (f *files) Remove(name string) error {
	return asGiven(os.Remove(f.Abs(name)), name, "") //nolint:forbidigo // toolctx.Files implementation
}

// RemoveAll implements toolctx.Files.
func (f *files) RemoveAll(name string) error {
	return asGiven(os.RemoveAll(f.Abs(name)), name, "") //nolint:forbidigo // toolctx.Files implementation
}

// Rename implements toolctx.Files. Renaming a regular file over newName
// publishes new contents there, which is recorded as an edit of newName.
func (f *files) Rename(oldName, newName string) error {
	oldPath, newPath := f.Abs(oldName), f.Abs(newName)
	rename := func() error {
		return os.Rename(oldPath, newPath) //nolint:forbidigo // toolctx.Files implementation
	}
	info, err := os.Lstat(oldPath) //nolint:forbidigo // toolctx.Files implementation
	if err != nil || !info.Mode().IsRegular() || !f.recording(newPath) {
		return asGiven(rename(), oldName, newName)
	}
	f.job.spec.Edits.Record(newPath, func() {
		err = rename()
	})
	return asGiven(err, oldName, newName)
}

// Symlink implements toolctx.Files. The target is stored as given.
func (f *files) Symlink(target, name string) error {
	return asGiven(os.Symlink(target, f.Abs(name)), target, name) //nolint:forbidigo // toolctx.Files implementation
}

// Link implements toolctx.Files.
func (f *files) Link(oldName, newName string) error {
	return asGiven(os.Link(f.Abs(oldName), f.Abs(newName)), oldName, newName) //nolint:forbidigo // toolctx.Files implementation
}

// Chmod implements toolctx.Files.
func (f *files) Chmod(name string, mode fs.FileMode) error {
	return asGiven(os.Chmod(f.Abs(name), mode), name, "") //nolint:forbidigo // toolctx.Files implementation
}

// Chown implements toolctx.Files.
func (f *files) Chown(name string, uid, gid int) error {
	return asGiven(os.Chown(f.Abs(name), uid, gid), name, "") //nolint:forbidigo // toolctx.Files implementation
}

// Lchown implements toolctx.Files.
func (f *files) Lchown(name string, uid, gid int) error {
	return asGiven(os.Lchown(f.Abs(name), uid, gid), name, "") //nolint:forbidigo // toolctx.Files implementation
}

// Chtimes implements toolctx.Files.
func (f *files) Chtimes(name string, atime, mtime time.Time) error {
	return asGiven(os.Chtimes(f.Abs(name), atime, mtime), name, "") //nolint:forbidigo // toolctx.Files implementation
}

// Truncate implements toolctx.Files. Truncating a regular file is recorded
// as an edit.
func (f *files) Truncate(name string, size int64) error {
	path := f.Abs(name)
	truncate := func() error {
		return os.Truncate(path, size) //nolint:forbidigo // toolctx.Files implementation
	}
	if !f.recording(path) {
		return asGiven(truncate(), name, "")
	}
	var err error
	f.job.spec.Edits.Record(path, func() {
		err = truncate()
	})
	return asGiven(err, name, "")
}

// CreateTemp implements toolctx.Files. Writes to a temporary file are not
// edits; renaming it over a file is.
func (f *files) CreateTemp(dir, pattern string) (toolctx.File, error) {
	file, err := waitForDescriptors(f.job.ctx, func() (*os.File, error) {
		return os.CreateTemp(f.Abs(dir), pattern) //nolint:forbidigo // toolctx.Files implementation
	})
	if err != nil {
		return nil, err
	}
	return f.track(&jobFile{file: file, name: file.Name(), path: file.Name()})
}

// MkdirTemp implements toolctx.Files.
func (f *files) MkdirTemp(dir, pattern string) (string, error) {
	return os.MkdirTemp(f.Abs(dir), pattern) //nolint:forbidigo // toolctx.Files implementation
}

// asGiven makes err name the paths as the caller gave them, not as
// resolved against the working directory.
func asGiven(err error, name, newName string) error {
	var pathErr *fs.PathError
	if errors.As(err, &pathErr) {
		return &fs.PathError{Op: pathErr.Op, Path: name, Err: pathErr.Err}
	}
	var linkErr *os.LinkError
	if errors.As(err, &linkErr) {
		return &os.LinkError{Op: linkErr.Op, Old: name, New: newName, Err: linkErr.Err}
	}
	return err
}

// jobFile is a file that a job opened. Writes to a regular file opened for
// writing are recorded against the path it was opened as, as long as that
// path still names it. It does not embed the os.File, whose other writing
// methods, such as ReadFrom, would bypass the recorder.
type jobFile struct {
	file *os.File
	job  *Job
	// name is the path as given, path the absolute one.
	name string
	path string
	// recorder is nil when writes are not recorded.
	recorder EditRecorder
}

var _ toolctx.File = (*jobFile)(nil)

// Name returns the name as given.
func (f *jobFile) Name() string { return f.name }

func (f *jobFile) Read(b []byte) (int, error) { return f.file.Read(b) }

func (f *jobFile) ReadAt(b []byte, off int64) (int, error) { return f.file.ReadAt(b, off) }

func (f *jobFile) Seek(offset int64, whence int) (int64, error) {
	return f.file.Seek(offset, whence)
}

func (f *jobFile) Stat() (fs.FileInfo, error) { return f.file.Stat() }

func (f *jobFile) ReadDir(n int) ([]fs.DirEntry, error) { return f.file.ReadDir(n) }

func (f *jobFile) Sync() error { return f.file.Sync() }

// SetReadDeadline lets the read builtin interrupt a read from a pipe or
// device that a redirection opened.
func (f *jobFile) SetReadDeadline(t time.Time) error { return f.file.SetReadDeadline(t) }

// recorded reports whether a write is recorded now. A write to a file that
// was renamed away or replaced does not edit the file now at its path.
func (f *jobFile) recorded() bool {
	if f.recorder == nil {
		return false
	}
	current, err := os.Stat(f.path) //nolint:forbidigo // toolctx.Files implementation
	if err != nil {
		return false
	}
	own, err := f.file.Stat()
	return err == nil && os.SameFile(current, own)
}

func (f *jobFile) Write(b []byte) (n int, err error) {
	if !f.recorded() {
		return f.file.Write(b)
	}
	f.recorder.Record(f.path, func() {
		n, err = f.file.Write(b)
	})
	return n, err
}

func (f *jobFile) WriteAt(b []byte, off int64) (n int, err error) {
	if !f.recorded() {
		return f.file.WriteAt(b, off)
	}
	f.recorder.Record(f.path, func() {
		n, err = f.file.WriteAt(b, off)
	})
	return n, err
}

func (f *jobFile) Truncate(size int64) (err error) {
	if !f.recorded() {
		return f.file.Truncate(size)
	}
	f.recorder.Record(f.path, func() {
		err = f.file.Truncate(size)
	})
	return err
}

// Close closes the file and stops tracking it.
func (f *jobFile) Close() error {
	f.job.untrack(f)
	return f.file.Close()
}

// streamFile is a command's descriptor opened by its path, such as
// "/dev/stdout". Closing it leaves the descriptor open.
type streamFile struct {
	name string
	d    interp.Descriptor
}

var _ toolctx.File = (*streamFile)(nil)

func (f *streamFile) Read(b []byte) (int, error) {
	if f.d.Reader == nil {
		return 0, &fs.PathError{Op: "read", Path: f.name, Err: syscall.EBADF}
	}
	return f.d.Reader.Read(b)
}

func (f *streamFile) Write(b []byte) (int, error) {
	if f.d.Writer == nil {
		return 0, &fs.PathError{Op: "write", Path: f.name, Err: syscall.EBADF}
	}
	return f.d.Writer.Write(b)
}

func (f *streamFile) ReadAt([]byte, int64) (int, error) {
	return 0, &fs.PathError{Op: "read", Path: f.name, Err: syscall.ESPIPE}
}

func (f *streamFile) WriteAt([]byte, int64) (int, error) {
	return 0, &fs.PathError{Op: "write", Path: f.name, Err: syscall.ESPIPE}
}

func (f *streamFile) Seek(int64, int) (int64, error) {
	return 0, &fs.PathError{Op: "seek", Path: f.name, Err: syscall.ESPIPE}
}

func (f *streamFile) Close() error { return nil }

func (f *streamFile) Name() string { return f.name }

// Stat describes the file behind the descriptor, or a pipe for a stream
// that is not a file.
func (f *streamFile) Stat() (fs.FileInfo, error) {
	for _, end := range [...]any{f.d.Reader, f.d.Writer} {
		if file, ok := end.(interface{ Stat() (fs.FileInfo, error) }); ok {
			return file.Stat()
		}
	}
	return streamInfo{name: filepath.Base(f.name)}, nil
}

func (f *streamFile) ReadDir(int) ([]fs.DirEntry, error) {
	return nil, &fs.PathError{Op: "readdirent", Path: f.name, Err: syscall.ENOTDIR}
}

func (f *streamFile) Sync() error {
	return &fs.PathError{Op: "sync", Path: f.name, Err: syscall.EINVAL}
}

func (f *streamFile) Truncate(int64) error {
	return &fs.PathError{Op: "truncate", Path: f.name, Err: syscall.EINVAL}
}

// streamInfo describes a stream that is not a file as a pipe.
type streamInfo struct{ name string }

func (i streamInfo) Name() string       { return i.name }
func (i streamInfo) Size() int64        { return 0 }
func (i streamInfo) Mode() fs.FileMode  { return fs.ModeNamedPipe | 0o600 }
func (i streamInfo) ModTime() time.Time { return time.Time{} }
func (i streamInfo) IsDir() bool        { return false }
func (i streamInfo) Sys() any           { return nil }
