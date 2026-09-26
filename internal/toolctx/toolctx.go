// Package toolctx defines how a standard utility runs inside a shell job.
//
// A utility is a Go function that the runner calls in its own process. It
// receives everything a process would take from the operating system through
// an Invocation: its working directory, environment, standard streams, umask,
// cancellation, the filesystem, and a way to run another command. A utility
// never reads or changes process-global state; the job's owner supplies these
// values, so concurrent jobs cannot see each other's directory, environment or
// streams, and every write goes through the job's edit recorder.
//
// Each utility family package exports its utilities as
//
//	var Utilities = map[string]toolctx.Utility{...}
//
// and internal/tools lists the families. Files.OpenFile takes the os.O_*
// flags.
//
// The design is described in docs/demi-next/runner.md § Shell jobs and
// docs/demi-next/edit-tracking.md § Recording actual writes.
package toolctx

import (
	"context"
	"io"
	"io/fs"
	"time"
)

// Utility runs one invocation and returns its exit status. args[0] is the
// name the utility was called by. A utility reports its own errors on
// inv.Stderr, as the program it replaces would, and returns a non-zero status.
type Utility func(inv *Invocation, args []string) int

// Invocation is one run of a utility inside a shell job.
type Invocation struct {
	// Context ends when the job is cancelled. A utility checks it in every
	// loop that can run long and stops at once when it is done.
	Context context.Context
	// Dir is the absolute working directory. Files resolves relative names
	// against it; a utility never resolves them itself.
	Dir string
	// Env is the job's environment as the utility sees it.
	Env Env
	// Stdin, Stdout and Stderr are the standard streams. After cancellation
	// every read and write on them returns an error.
	Stdin  io.Reader
	Stdout io.Writer
	Stderr io.Writer
	// StdinKind says what Stdin is connected to.
	StdinKind StdinKind
	// Umask applies to every file and directory the utility creates.
	Umask fs.FileMode
	// Files is the only way a utility reaches the filesystem.
	Files Files
	// Run runs another command through the job, as xargs, find -exec and env
	// do: a declared root, another utility or an external program.
	Run func(ctx context.Context, cmd Command) (int, error)
}

// StdinKind says what a utility's standard input is connected to. Some
// utilities decide on it: rg searches the working directory when there is no
// input and searches its input otherwise.
type StdinKind int

const (
	// StdinNone: no input; reads see end of file at once, as from /dev/null.
	StdinNone StdinKind = iota
	// StdinPipe: a pipe from another command or from the caller's live input.
	StdinPipe
	// StdinFile: a regular file, as with `< file`.
	StdinFile
)

// Command is a command that a utility runs through its job.
type Command struct {
	Args   []string
	Env    Env
	Dir    string
	Stdin  io.Reader
	Stdout io.Writer
	Stderr io.Writer
}

// Env is a read-only view of an environment.
type Env interface {
	// Get returns the value of name and whether it is set.
	Get(name string) (string, bool)
	// Each calls fn for every variable in a stable order until fn returns false.
	Each(fn func(name, value string) bool)
}

// Files is the filesystem as seen by one invocation. Relative names resolve
// against Invocation.Dir. Every mutation is observed by the job's edit
// recorder; errors are *fs.PathError values naming the path as given.
type Files interface {
	Open(name string) (File, error)
	OpenFile(name string, flag int, perm fs.FileMode) (File, error)
	Stat(name string) (fs.FileInfo, error)
	Lstat(name string) (fs.FileInfo, error)
	ReadDir(name string) ([]fs.DirEntry, error)
	Readlink(name string) (string, error)
	// Abs returns the absolute, cleaned form of name.
	Abs(name string) string

	Mkdir(name string, perm fs.FileMode) error
	MkdirAll(name string, perm fs.FileMode) error
	Remove(name string) error
	RemoveAll(name string) error
	Rename(oldName, newName string) error
	Symlink(target, name string) error
	Link(oldName, newName string) error
	Chmod(name string, mode fs.FileMode) error
	Chown(name string, uid, gid int) error
	Lchown(name string, uid, gid int) error
	Chtimes(name string, atime, mtime time.Time) error
	Truncate(name string, size int64) error
	// CreateTemp creates a new file in dir (Invocation.Dir when empty),
	// like os.CreateTemp.
	CreateTemp(dir, pattern string) (File, error)
	// MkdirTemp creates a new directory, like os.MkdirTemp.
	MkdirTemp(dir, pattern string) (string, error)
}

// File is an open file. Writes to a file opened for writing are recorded
// against the path it was opened as.
type File interface {
	io.Reader
	io.ReaderAt
	io.Writer
	io.WriterAt
	io.Seeker
	io.Closer
	Name() string
	Stat() (fs.FileInfo, error)
	ReadDir(n int) ([]fs.DirEntry, error)
	Sync() error
	Truncate(size int64) error
}
