package shell

import (
	"context"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"runtime/debug"
	"strings"

	"mvdan.cc/sh/v3/expand"
	"mvdan.cc/sh/v3/interp"

	"github.com/wspl/demi/internal/toolctx"
)

// unrecordedUtilities are the utilities whose writes are not edits
// (docs/demi-next/edit-tracking.md § Scope): cp copies, mv moves, and mktemp
// creates an empty file.
var unrecordedUtilities = map[string]bool{
	"cp":     true,
	"mv":     true,
	"mktemp": true,
}

// command is one command that a job runs, from the shell or from a utility's
// Invocation.Run.
type command struct {
	args    []string
	dir     string
	env     environ
	umask   fs.FileMode
	streams streams
}

// openHandler implements interp.OpenHandlerFunc: redirections open files
// through the job's filesystem, which records writes.
func (j *Job) openHandler(ctx context.Context, path string, flag int, perm fs.FileMode) (io.ReadWriteCloser, error) {
	hc := interp.HandlerCtx(ctx)
	files := &files{job: j, dir: hc.Dir, umask: hc.Umask, record: true}
	return files.OpenFile(path, flag, perm)
}

// execMiddleware implements the job's exec handler; it never calls next.
func (j *Job) execMiddleware(next interp.ExecHandlerFunc) interp.ExecHandlerFunc {
	return func(ctx context.Context, args []string) error {
		hc := interp.HandlerCtx(ctx)
		cmd := command{
			args:  args,
			dir:   hc.Dir,
			env:   exported(hc.Env),
			umask: hc.Umask,
			streams: streams{
				stdin:       hc.Stdin,
				stdout:      hc.Stdout,
				stderr:      hc.Stderr,
				descriptors: hc.Descriptors,
			},
		}
		code, err := j.runCommand(ctx, cmd)
		if err != nil {
			fmt.Fprintf(hc.Stderr, "%s: %v\n", args[0], commandError(err))
		}
		if code != 0 {
			return interp.ExitStatus(code)
		}
		return nil
	}
}

// commandError describes why a command could not run, as Bash does.
func commandError(err error) string {
	switch {
	case errors.Is(err, errNotFound):
		return "command not found"
	case errors.Is(err, fs.ErrNotExist):
		return "No such file or directory"
	}
	return err.Error()
}

// errNotFound means that no declared root, utility or program has the name.
var errNotFound = errors.New("command not found")

// runCommand runs a declared root, a utility or an external program, in that
// order, and returns its exit status. An error means the command could not
// run; the status is then 127 when nothing has the name and 126 otherwise.
func (j *Job) runCommand(ctx context.Context, cmd command) (int, error) {
	name := cmd.args[0]
	if commands := j.spec.Commands; commands != nil && commands.Dispatcher.Declared(name) {
		return j.dispatch(ctx, cmd)
	}
	if utility, ok := j.shell.config.Utilities[name]; ok {
		return j.runUtility(ctx, utility, cmd), nil
	}
	path, err := interp.LookPathDir(cmd.dir, expand.ListEnviron(cmd.env...), name)
	switch {
	case err == nil:
		return j.runExternal(ctx, path, cmd)
	case !strings.ContainsRune(name, '/'):
		return 127, errNotFound
	case errors.Is(err, fs.ErrNotExist):
		return 127, err
	default:
		// A directory, or a file without permission to execute it.
		return 126, err
	}
}

// dispatch runs a declared root through the job's dispatcher.
func (j *Job) dispatch(ctx context.Context, cmd command) (int, error) {
	return j.spec.Commands.Dispatcher.Dispatch(ctx, toolctx.Command{
		Args:   cmd.args,
		Env:    cmd.env,
		Dir:    cmd.dir,
		Stdin:  cmd.streams.stdin,
		Stdout: cmd.streams.stdout,
		Stderr: cmd.streams.stderr,
	})
}

// runUtility runs a utility on its own goroutine and waits for it. A panic
// in the utility is reported on its standard error with status 2.
func (j *Job) runUtility(ctx context.Context, utility toolctx.Utility, cmd command) int {
	inv := &toolctx.Invocation{
		Context: ctx,
		Dir:     cmd.dir,
		Env:     cmd.env,
		Stdin:   guardedReader{ctx: ctx, r: cmd.streams.stdin},
		Stdout:  guardedWriter{ctx: ctx, w: cmd.streams.stdout},
		Stderr:  guardedWriter{ctx: ctx, w: cmd.streams.stderr},
		Umask:   cmd.umask,
		Files: &files{
			job:     j,
			dir:     cmd.dir,
			umask:   cmd.umask,
			record:  !unrecordedUtilities[cmd.args[0]],
			streams: cmd.streams,
		},
	}
	inv.Run = func(ctx context.Context, run toolctx.Command) (int, error) {
		return j.runCommand(ctx, cmd.nested(run))
	}
	code := make(chan int, 1)
	j.goTask(func() {
		defer func() {
			if value := recover(); value != nil {
				fmt.Fprintf(inv.Stderr, "%s: internal error: %v\n%s", cmd.args[0], value, debug.Stack())
				code <- 2
			}
		}()
		code <- utility(inv, cmd.args)
	})
	return <-code
}

// nested is the command that a utility runs through Invocation.Run: it
// keeps this command's directory, environment and streams unless run
// replaces them.
func (cmd command) nested(run toolctx.Command) command {
	nested := cmd
	nested.args = run.Args
	if run.Dir != "" {
		nested.dir = run.Dir
	}
	if run.Env != nil {
		nested.env = fromToolEnv(run.Env)
	}
	if run.Stdin != nil {
		nested.streams.stdin = run.Stdin
	}
	if run.Stdout != nil {
		nested.streams.stdout = run.Stdout
	}
	if run.Stderr != nil {
		nested.streams.stderr = run.Stderr
	}
	return nested
}

// guardedReader fails every read once the command's context has ended, as
// toolctx.Invocation promises. A nil reader is a closed standard input.
type guardedReader struct {
	ctx context.Context
	r   io.Reader
}

func (g guardedReader) Read(b []byte) (int, error) {
	if err := g.ctx.Err(); err != nil {
		return 0, err
	}
	if g.r == nil {
		return 0, &fs.PathError{Op: "read", Path: "/dev/stdin", Err: os.ErrClosed}
	}
	return g.r.Read(b)
}

// guardedWriter fails every write once the command's context has ended.
type guardedWriter struct {
	ctx context.Context
	w   io.Writer
}

func (g guardedWriter) Write(b []byte) (int, error) {
	if err := g.ctx.Err(); err != nil {
		return 0, err
	}
	return g.w.Write(b)
}

// unguardedReader returns the stream a guard wraps, so that an external
// program run by a utility receives the stream itself.
func unguardedReader(stream io.Reader) io.Reader {
	if guard, ok := stream.(guardedReader); ok {
		return guard.r
	}
	return stream
}

// unguardedWriter is unguardedReader for output streams.
func unguardedWriter(stream io.Writer) io.Writer {
	if guard, ok := stream.(guardedWriter); ok {
		return guard.w
	}
	return stream
}
