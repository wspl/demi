// Package toolctxtest runs utilities in tests: an Invocation whose files live
// in a directory of the test's choice, with an environment from a map and
// standard streams in memory. It records nothing; edit recording belongs to
// the runner's job and is tested there.
package toolctxtest

import (
	"bytes"
	"context"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/wspl/demi/internal/toolctx"
)

// Env is an environment held in a map.
type Env map[string]string

// Get implements toolctx.Env.
func (e Env) Get(name string) (string, bool) {
	value, ok := e[name]
	return value, ok
}

// Each implements toolctx.Env in name order.
func (e Env) Each(fn func(name, value string) bool) {
	names := make([]string, 0, len(e))
	for name := range e {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		if !fn(name, e[name]) {
			return
		}
	}
}

// Result is what one run produced.
type Result struct {
	Code   int
	Stdout string
	Stderr string
}

// Runner runs utilities with Dir as the working directory.
type Runner struct {
	Dir   string
	Env   Env
	Umask fs.FileMode
	// Utilities are the commands Invocation.Run can reach, by name.
	Utilities map[string]toolctx.Utility
}

// Run runs utility with args (args[0] is its name) and stdin, which reaches
// the utility as a pipe.
func (r *Runner) Run(ctx context.Context, utility toolctx.Utility, args []string, stdin string) Result {
	return r.RunInput(ctx, utility, args, strings.NewReader(stdin), toolctx.StdinPipe)
}

// RunInput runs utility with an explicit standard input and its kind.
func (r *Runner) RunInput(ctx context.Context, utility toolctx.Utility, args []string, stdin io.Reader, kind toolctx.StdinKind) Result {
	var stdout, stderr bytes.Buffer
	inv := r.invocation(ctx, r.Dir, r.Env, stdin, &stdout, &stderr)
	inv.StdinKind = kind
	code := utility(inv, args)
	return Result{Code: code, Stdout: stdout.String(), Stderr: stderr.String()}
}

func (r *Runner) invocation(ctx context.Context, dir string, env toolctx.Env, stdin io.Reader, stdout, stderr io.Writer) *toolctx.Invocation {
	umask := r.Umask
	if umask == 0 {
		umask = 0o022
	}
	return &toolctx.Invocation{
		Context: ctx,
		Dir:     dir,
		Env:     env,
		Stdin:   stdin,
		Stdout:  stdout,
		Stderr:  stderr,
		Umask:   umask,
		Files:   Files{Dir: dir},
		Run:     r.run,
	}
}

func (r *Runner) run(ctx context.Context, cmd toolctx.Command) (int, error) {
	utility, ok := r.Utilities[cmd.Args[0]]
	if !ok {
		return 127, &fs.PathError{Op: "run", Path: cmd.Args[0], Err: fs.ErrNotExist}
	}
	dir := cmd.Dir
	if dir == "" {
		dir = r.Dir
	}
	var env toolctx.Env = r.Env
	if cmd.Env != nil {
		env = cmd.Env
	}
	stdin := cmd.Stdin
	kind := toolctx.StdinPipe
	if stdin == nil {
		stdin = strings.NewReader("")
		kind = toolctx.StdinNone
	}
	inv := r.invocation(ctx, dir, env, stdin, cmd.Stdout, cmd.Stderr)
	inv.StdinKind = kind
	return utility(inv, cmd.Args), nil
}

// Files implements toolctx.Files on the real filesystem, resolving relative
// names against Dir.
type Files struct {
	Dir string
}

var _ toolctx.Files = Files{}

// Abs implements toolctx.Files.
func (f Files) Abs(name string) string {
	if filepath.IsAbs(name) {
		return filepath.Clean(name)
	}
	return filepath.Join(f.Dir, name)
}

// Open implements toolctx.Files.
func (f Files) Open(name string) (toolctx.File, error) {
	return f.file(os.Open(f.Abs(name)))
}

// OpenFile implements toolctx.Files.
func (f Files) OpenFile(name string, flag int, perm fs.FileMode) (toolctx.File, error) {
	return f.file(os.OpenFile(f.Abs(name), flag, perm))
}

// CreateTemp implements toolctx.Files.
func (f Files) CreateTemp(dir, pattern string) (toolctx.File, error) {
	return f.file(os.CreateTemp(f.Abs(dir), pattern))
}

// file converts a typed nil *os.File into a nil interface on error.
func (Files) file(file *os.File, err error) (toolctx.File, error) {
	if err != nil {
		return nil, err
	}
	return file, nil
}

// Stat implements toolctx.Files.
func (f Files) Stat(name string) (fs.FileInfo, error) { return os.Stat(f.Abs(name)) }

// Lstat implements toolctx.Files.
func (f Files) Lstat(name string) (fs.FileInfo, error) { return os.Lstat(f.Abs(name)) }

// ReadDir implements toolctx.Files.
func (f Files) ReadDir(name string) ([]fs.DirEntry, error) { return os.ReadDir(f.Abs(name)) }

// Readlink implements toolctx.Files.
func (f Files) Readlink(name string) (string, error) { return os.Readlink(f.Abs(name)) }

// Mkdir implements toolctx.Files.
func (f Files) Mkdir(name string, perm fs.FileMode) error { return os.Mkdir(f.Abs(name), perm) }

// MkdirAll implements toolctx.Files.
func (f Files) MkdirAll(name string, perm fs.FileMode) error { return os.MkdirAll(f.Abs(name), perm) }

// MkdirTemp implements toolctx.Files.
func (f Files) MkdirTemp(dir, pattern string) (string, error) {
	return os.MkdirTemp(f.Abs(dir), pattern)
}

// Remove implements toolctx.Files.
func (f Files) Remove(name string) error { return os.Remove(f.Abs(name)) }

// RemoveAll implements toolctx.Files.
func (f Files) RemoveAll(name string) error { return os.RemoveAll(f.Abs(name)) }

// Rename implements toolctx.Files.
func (f Files) Rename(oldName, newName string) error {
	return os.Rename(f.Abs(oldName), f.Abs(newName))
}

// Symlink implements toolctx.Files. The target is stored as given.
func (f Files) Symlink(target, name string) error { return os.Symlink(target, f.Abs(name)) }

// Link implements toolctx.Files.
func (f Files) Link(oldName, newName string) error { return os.Link(f.Abs(oldName), f.Abs(newName)) }

// Chmod implements toolctx.Files.
func (f Files) Chmod(name string, mode fs.FileMode) error { return os.Chmod(f.Abs(name), mode) }

// Chown implements toolctx.Files.
func (f Files) Chown(name string, uid, gid int) error { return os.Chown(f.Abs(name), uid, gid) }

// Lchown implements toolctx.Files.
func (f Files) Lchown(name string, uid, gid int) error { return os.Lchown(f.Abs(name), uid, gid) }

// Chtimes implements toolctx.Files.
func (f Files) Chtimes(name string, atime, mtime time.Time) error {
	return os.Chtimes(f.Abs(name), atime, mtime)
}

// Truncate implements toolctx.Files.
func (f Files) Truncate(name string, size int64) error { return os.Truncate(f.Abs(name), size) }
