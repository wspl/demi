package hosttest

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"reflect"
	"strings"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
)

// ConformanceCase is one observable behavior every Host must provide.
// Run waits only for process or IO events, with its caller's context as hang guard.
type ConformanceCase struct {
	Name string
	Run  func(context.Context) error
}

// ConformanceCases returns Host conformance cases using a fresh absolute root,
// which must also be the Host's default directory. PATH must locate sh, printf,
// sleep, printenv, cat and echo. No real model or network is used.
func ConformanceCases(h host.Host, root, path string) []ConformanceCase {
	c := conformance{h: h, root: root, env: host.SpawnEnv{Mode: host.Exactly, Values: map[string]*string{"PATH": &path}}}
	return []ConformanceCase{
		{"host: the default working directory is absolute; the identity names a host", func(context.Context) error {
			if !strings.HasPrefix(h.DefaultCWD(), "/") || h.DefaultCWD() != root || h.Identity().Hostname == "" {
				return fmt.Errorf("default directory or identity differs: %q, %+v", h.DefaultCWD(), h.Identity())
			}
			return nil
		}},
		{"process: spawn captures stdout and the exit code", func(ctx context.Context) error {
			o, err := c.run(ctx, "printf", []string{"hello\\n"}, nil)
			if err != nil {
				return err
			}
			return expect(o, processOutput{stdout: "hello\n", end: host.ProcessEnd{Kind: host.ProcessExited}}, "stdout and end")
		}},
		{"process: stdout and stderr are apart; a nonzero exit is reported", func(ctx context.Context) error {
			o, err := c.run(ctx, "sh", []string{"-c", "echo out; echo err >&2; exit 3"}, nil)
			if err != nil {
				return err
			}
			return expect(o, processOutput{stdout: "out\n", stderr: "err\n", end: host.ProcessEnd{Kind: host.ProcessExited, ExitCode: 3}}, "output and end")
		}},
		{"process: stdin reaches the child and ends when closed", c.stdin},
		{"process: terminating a process ends it with SIGTERM", c.terminate},
		{"process: a child receives exactly the environment it was given", func(ctx context.Context) error {
			o, err := c.run(ctx, "printenv", []string{"HOME"}, nil)
			if err != nil {
				return err
			}
			return expect(o, processOutput{end: host.ProcessEnd{Kind: host.ProcessExited, ExitCode: 1}}, "unset HOME")
		}},
		{"process: the working directory is honoured", c.cwd},
		{"process: a missing program never starts: executable_not_found", func(ctx context.Context) error {
			o, err := c.run(ctx, "definitely-not-a-host-binary", nil, nil)
			if err != nil {
				return err
			}
			return expectSpawn(o.end, host.ExecutableNotFound)
		}},
		{"process: a missing working directory is cwd_unusable, not a missing program", func(ctx context.Context) error {
			missing := root + "/never-created"
			o, err := c.run(ctx, "echo", []string{"ok"}, &missing)
			if err != nil {
				return err
			}
			return expectSpawn(o.end, host.CWDUnusable)
		}},
		{"fs: write, replace, read, list, stat, exists, remove; relative paths", c.basic},
		{"fs: parents, recursive copy, move, recursive remove, force", c.tree},
		{"fs: symlink, readlink, lstat, realpath, link, chmod, utimes", c.links},
		{"fs: a stream reads the whole file or one byte range; a missing file fails first", c.stream},
		{"fs: a write takes a stream; one that fails leaves the file as it was", c.writeStream},
		{"fs: errors carry the operating system's codes", c.codes},
		{"fs and process share one namespace", c.namespace},
	}
}

type conformance struct {
	h    host.Host
	root string
	env  host.SpawnEnv
}
type processOutput struct {
	stdout, stderr string
	end            host.ProcessEnd
}

// spawn starts a conformance program with its exact environment.
func (c conformance) spawn(ctx context.Context, command string, args []string, cwd *string) (*host.StartedProcess, error) {
	return c.h.Process().Spawn(ctx, host.SpawnRequest{Command: command, Args: args, CWD: cwd, Env: c.env})
}

// run owns, collects and joins one conformance program.
func (c conformance) run(ctx context.Context, command string, args []string, cwd *string) (out processOutput, err error) {
	p, err := c.spawn(ctx, command, args, cwd)
	if err != nil {
		return out, err
	}
	defer cleanupProcess(ctx, p, &err)
	return collect(ctx, p)
}

// cleanupProcess releases control and joins the process even when collection failed.
func cleanupProcess(ctx context.Context, p *host.StartedProcess, err *error) {
	cleanup := context.WithoutCancel(ctx)
	closeErr := p.Control.Close(cleanup)
	_, waitErr := p.Wait(cleanup)
	*err = errors.Join(*err, closeErr, waitErr)
}

// collect drains a child's ordered streams before awaiting its end.
func collect(ctx context.Context, p *host.StartedProcess) (processOutput, error) {
	var stdout, stderr bytes.Buffer
	for {
		chunk, err := p.Output.Next(ctx)
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return processOutput{}, err
		}
		if chunk.Stream == "stdout" {
			stdout.Write(chunk.Bytes)
		} else {
			stderr.Write(chunk.Bytes)
		}
	}
	end, err := p.Wait(ctx)
	return processOutput{stdout: strings.ToValidUTF8(stdout.String(), "�"), stderr: strings.ToValidUTF8(stderr.String(), "�"), end: end}, err
}

// stdin proves delivery and explicit closure of the input stream.
func (c conformance) stdin(ctx context.Context) (err error) {
	p, err := c.spawn(ctx, "sh", []string{"-c", `IFS= read -r line; printf '%s' "$line"`}, nil)
	if err != nil {
		return err
	}
	defer cleanupProcess(ctx, p, &err)
	if err = p.Control.WriteStdin(ctx, []byte("from stdin\n")); err != nil {
		return err
	}
	if err = p.Control.CloseStdin(ctx); err != nil {
		return err
	}
	out, err := collect(ctx, p)
	if err != nil {
		return err
	}
	return expect(out, processOutput{stdout: "from stdin", end: host.ProcessEnd{Kind: host.ProcessExited}}, "stdin and end")
}

// terminate proves named signal delivery.
func (c conformance) terminate(ctx context.Context) (err error) {
	p, err := c.spawn(ctx, "sleep", []string{"10"}, nil)
	if err != nil {
		return err
	}
	defer cleanupProcess(ctx, p, &err)
	if err = p.Control.Kill(ctx, host.Terminate); err != nil {
		return err
	}
	out, err := collect(ctx, p)
	if err != nil {
		return err
	}
	return expect(out.end, host.ProcessEnd{Kind: host.ProcessSignalled, Signal: "SIGTERM"}, "signal end")
}

// cwd proves an explicitly selected working directory.
func (c conformance) cwd(ctx context.Context) error {
	directory := c.root + "/cwd-honoured"
	if err := c.h.FS().Mkdir(ctx, directory, host.MkdirOptions{Recursive: true}); err != nil {
		return err
	}
	out, err := c.run(ctx, "sh", []string{"-c", "pwd"}, &directory)
	if err != nil {
		return err
	}
	if err = expect(out.end, host.ProcessEnd{Kind: host.ProcessExited}, "pwd end"); err != nil {
		return err
	}
	if !strings.HasSuffix(strings.TrimRight(out.stdout, "\n"), "/cwd-honoured") {
		return fmt.Errorf("pwd is outside the directory: %q", out.stdout)
	}
	return nil
}

// basic proves relative paths and the fundamental filesystem lifecycle.
func (c conformance) basic(ctx context.Context) error {
	fs := c.h.FS()
	if err := fs.Mkdir(ctx, "basic/src", host.MkdirOptions{Recursive: true}); err != nil {
		return err
	}
	file := c.root + "/basic/src/file.txt"
	if err := fs.WriteFile(ctx, "basic/src/file.txt", fileBytes("first\n"), host.WriteOptions{}); err != nil {
		return err
	}
	if err := fs.WriteFile(ctx, file, fileBytes("hello\ntail\n"), host.WriteOptions{}); err != nil {
		return err
	}
	if err := c.readEquals(ctx, file, "hello\ntail\n"); err != nil {
		return err
	}
	entries, err := fs.ReadDir(ctx, "basic/src")
	if err != nil {
		return err
	}
	if err = expect(entries, []host.DirEntry{{Name: "file.txt", Kind: host.File}}, "listing"); err != nil {
		return err
	}
	stat, err := fs.Stat(ctx, file)
	if err != nil {
		return err
	}
	if stat.Kind != host.File || stat.Size != 11 || stat.Modified <= core.UnixEpoch {
		return fmt.Errorf("file metadata: %+v", stat)
	}
	stat, err = fs.Stat(ctx, "basic/src")
	if err != nil {
		return err
	}
	if stat.Kind != host.Directory {
		return fmt.Errorf("directory metadata: %+v", stat)
	}
	if err = fs.Rm(ctx, file, host.RmOptions{Force: true}); err != nil {
		return err
	}
	if err = c.exists(ctx, file, false); err != nil {
		return err
	}
	return c.exists(ctx, "basic/src", true)
}

// tree proves parent creation and recursive filesystem operations.
func (c conformance) tree(ctx context.Context) error {
	fs := c.h.FS()
	root := c.root + "/tree"
	if err := fs.Mkdir(ctx, root+"/a/b", host.MkdirOptions{Recursive: true}); err != nil {
		return err
	}
	if err := fs.WriteFile(ctx, root+"/a/b/f.txt", fileBytes("x"), host.WriteOptions{}); err != nil {
		return err
	}
	if err := fs.WriteFile(ctx, root+"/a/new/dir/g.txt", fileBytes("y"), host.WriteOptions{CreateParents: true}); err != nil {
		return err
	}
	if err := c.readEquals(ctx, root+"/a/new/dir/g.txt", "y"); err != nil {
		return err
	}
	if err := fs.Cp(ctx, root+"/a", root+"/c", host.CpOptions{Recursive: true}); err != nil {
		return err
	}
	for _, file := range []string{"/c/b/f.txt", "/a/b/f.txt"} {
		if err := c.readEquals(ctx, root+file, "x"); err != nil {
			return err
		}
	}
	if err := fs.Mv(ctx, root+"/c", root+"/d"); err != nil {
		return err
	}
	if err := c.exists(ctx, root+"/c", false); err != nil {
		return err
	}
	if err := c.readEquals(ctx, root+"/d/b/f.txt", "x"); err != nil {
		return err
	}
	if err := fs.Rm(ctx, root+"/d", host.RmOptions{Recursive: true}); err != nil {
		return err
	}
	if err := c.exists(ctx, root+"/d", false); err != nil {
		return err
	}
	if err := fs.Rm(ctx, root+"/absent", host.RmOptions{Force: true}); err != nil {
		return err
	}
	return expectCode(fs.Rm(ctx, root+"/absent", host.RmOptions{}), "ENOENT", "missing remove")
}

// links proves link identity, resolution, modes and timestamps.
func (c conformance) links(ctx context.Context) error {
	fs := c.h.FS()
	root := c.root + "/links"
	if err := fs.Mkdir(ctx, root, host.MkdirOptions{Recursive: true}); err != nil {
		return err
	}
	if err := fs.WriteFile(ctx, root+"/target.txt", fileBytes("t"), host.WriteOptions{}); err != nil {
		return err
	}
	if err := fs.Symlink(ctx, "target.txt", root+"/link"); err != nil {
		return err
	}
	target, err := fs.Readlink(ctx, root+"/link")
	if err != nil {
		return err
	}
	if err = expect(target, "target.txt", "readlink"); err != nil {
		return err
	}
	stat, err := fs.Lstat(ctx, root+"/link")
	if err != nil {
		return err
	}
	if err = expect(stat.Kind, host.Symlink, "lstat"); err != nil {
		return err
	}
	stat, err = fs.Stat(ctx, root+"/link")
	if err != nil {
		return err
	}
	if err = expect(stat.Kind, host.File, "stat follows"); err != nil {
		return err
	}
	resolved, err := fs.Realpath(ctx, root+"/link")
	if err != nil {
		return err
	}
	if !strings.HasSuffix(resolved, "/links/target.txt") {
		return fmt.Errorf("realpath did not resolve: %q", resolved)
	}
	entries, err := fs.ReadDir(ctx, root)
	if err != nil {
		return err
	}
	found := false
	for _, entry := range entries {
		if entry.Name == "link" && entry.Kind == host.Symlink {
			found = true
		}
	}
	if !found {
		return fmt.Errorf("listing does not mark link: %+v", entries)
	}
	if err = fs.Link(ctx, root+"/target.txt", root+"/hard"); err != nil {
		return err
	}
	if err = c.readEquals(ctx, root+"/hard", "t"); err != nil {
		return err
	}
	if err = fs.Chmod(ctx, root+"/target.txt", 0600); err != nil {
		return err
	}
	stat, err = fs.Stat(ctx, root+"/target.txt")
	if err != nil {
		return err
	}
	if err = expect(stat.Mode&0777, uint32(0600), "chmod"); err != nil {
		return err
	}
	when, err := core.TimestampFromMillisecond(1600000000000)
	if err != nil {
		return err
	}
	if err = fs.Utimes(ctx, root+"/target.txt", when, when); err != nil {
		return err
	}
	stat, err = fs.Stat(ctx, root+"/target.txt")
	if err != nil {
		return err
	}
	return expect(stat.Modified, when, "utimes")
}

// stream proves ranged reads and failure before any bytes for a missing file.
func (c conformance) stream(ctx context.Context) error {
	fs := c.h.FS()
	file := c.root + "/stream/digits.txt"
	if err := fs.WriteFile(ctx, file, fileBytes("0123456789"), host.WriteOptions{CreateParents: true}); err != nil {
		return err
	}
	length := uint64(4)
	for _, test := range []struct {
		r    host.ByteRange
		want string
	}{{host.ByteRange{}, "0123456789"}, {host.ByteRange{Offset: 3, Length: &length}, "3456"}, {host.ByteRange{Offset: 7}, "789"}} {
		got, err := readRange(ctx, fs, file, test.r)
		if err != nil {
			return err
		}
		if err = expect(string(got), test.want, "ranged stream"); err != nil {
			return err
		}
	}
	stream, err := fs.ReadStream(ctx, c.root+"/stream/missing", host.ByteRange{})
	if stream != nil {
		if closeErr := stream.Close(context.WithoutCancel(ctx)); closeErr != nil {
			return closeErr
		}
	}
	return expectCode(err, "ENOENT", "missing stream")
}

// readRange owns and drains a Host byte stream.
func readRange(ctx context.Context, fs host.FS, file string, r host.ByteRange) (data []byte, err error) {
	stream, err := fs.ReadStream(ctx, file, r)
	if err != nil {
		return nil, err
	}
	defer func() { err = errors.Join(err, stream.Close(context.WithoutCancel(ctx))) }()
	buffer := make([]byte, 32768)
	for {
		n, readErr := stream.Read(ctx, buffer)
		data = append(data, buffer[:n]...)
		if errors.Is(readErr, io.EOF) {
			return data, nil
		}
		if readErr != nil {
			return nil, readErr
		}
	}
}

type failedReader struct{ err error }

func (r failedReader) Read([]byte) (int, error) { return 0, r.err }

// writeStream proves streamed writes are atomic and return the stream's own failure.
func (c conformance) writeStream(ctx context.Context) error {
	fs := c.h.FS()
	root := c.root + "/write-stream"
	if err := fs.Mkdir(ctx, root, host.MkdirOptions{Recursive: true}); err != nil {
		return err
	}
	file := root + "/joined.txt"
	if err := fs.WriteFile(ctx, file, host.FileContents{Stream: &readerStream{reader: io.MultiReader(strings.NewReader("ab"), strings.NewReader("cd"), strings.NewReader("ef"))}}, host.WriteOptions{}); err != nil {
		return err
	}
	if err := c.readEquals(ctx, file, "abcdef"); err != nil {
		return err
	}
	broken := &host.Error{Kind: host.Interrupted, Message: "the stream broke"}
	err := fs.WriteFile(ctx, file, host.FileContents{Stream: &readerStream{reader: io.MultiReader(strings.NewReader("partial"), failedReader{broken})}}, host.WriteOptions{})
	if !errors.Is(err, broken) {
		return fmt.Errorf("write did not return stream failure: %w", err)
	}
	if err = c.readEquals(ctx, file, "abcdef"); err != nil {
		return err
	}
	entries, err := fs.ReadDir(ctx, root)
	if err != nil {
		return err
	}
	if len(entries) != 1 || entries[0].Name != "joined.txt" {
		return fmt.Errorf("partial copy left: %+v", entries)
	}
	return nil
}

// codes proves operating system causes survive the Host boundary.
func (c conformance) codes(ctx context.Context) error {
	fs := c.h.FS()
	root := c.root + "/errors"
	if err := fs.Mkdir(ctx, root, host.MkdirOptions{Recursive: true}); err != nil {
		return err
	}
	_, err := fs.ReadFile(ctx, root+"/missing")
	if err = expectCode(err, "ENOENT", "read_file"); err != nil {
		return err
	}
	_, err = fs.Stat(ctx, root+"/missing")
	if err = expectCode(err, "ENOENT", "stat"); err != nil {
		return err
	}
	if err = expectCode(fs.Mkdir(ctx, root, host.MkdirOptions{}), "EEXIST", "mkdir exists"); err != nil {
		return err
	}
	_, err = fs.ReadDir(ctx, root+"/missing")
	return expectCode(err, "ENOENT", "read_dir")
}

// namespace proves filesystem and processes act on the same target paths.
func (c conformance) namespace(ctx context.Context) error {
	fs := c.h.FS()
	root := c.root + "/shared"
	if err := fs.Mkdir(ctx, root, host.MkdirOptions{Recursive: true}); err != nil {
		return err
	}
	if err := fs.WriteFile(ctx, root+"/seen.txt", fileBytes("seen by cat\n"), host.WriteOptions{}); err != nil {
		return err
	}
	out, err := c.run(ctx, "cat", []string{"seen.txt"}, &root)
	if err != nil {
		return err
	}
	if err = expect(out.stdout, "seen by cat\n", "cat output"); err != nil {
		return err
	}
	out, err = c.run(ctx, "sh", []string{"-c", "printf back > written.txt"}, &root)
	if err != nil {
		return err
	}
	if err = expect(out.end, host.ProcessEnd{Kind: host.ProcessExited}, "shell write"); err != nil {
		return err
	}
	return c.readEquals(ctx, root+"/written.txt", "back")
}

// readEquals checks a conformance file's complete contents.
func (c conformance) readEquals(ctx context.Context, file, want string) error {
	data, err := c.h.FS().ReadFile(ctx, file)
	if err != nil {
		return err
	}
	return expect(string(data), want, file)
}

// exists checks a conformance path's existence.
func (c conformance) exists(ctx context.Context, file string, want bool) error {
	found, err := c.h.FS().Exists(ctx, file)
	if err != nil {
		return err
	}
	return expect(found, want, file)
}

// expect reports a conformance mismatch with its observable purpose.
func expect(actual, want any, what string) error {
	if !reflect.DeepEqual(actual, want) {
		return fmt.Errorf("%s: expected %#v, got %#v", what, want, actual)
	}
	return nil
}

// expectCode checks an operating system cause through wrapped Host errors.
func expectCode(err error, code, what string) error {
	var failure *host.Error
	if errors.As(err, &failure) && failure.Kind == host.Failed && failure.Code == code {
		return nil
	}
	return fmt.Errorf("%s: expected %s, got %v", what, code, err)
}

// expectSpawn checks a startup failure without conflating it with an operation failure.
func expectSpawn(end host.ProcessEnd, kind host.SpawnErrorKind) error {
	if end.Kind == host.ProcessNotStarted && end.SpawnError != nil && end.SpawnError.Kind == kind {
		return nil
	}
	return fmt.Errorf("expected a %s spawn error, got %+v", kind, end)
}

// fileBytes supplies fixed conformance contents without a stream resource.
func fileBytes(text string) host.FileContents { return host.FileContents{Bytes: []byte(text)} }

// readerStream adapts finite in-memory readers for the streaming conformance cases.
// None of these readers block; real Host streams must support context cancellation.
type readerStream struct{ reader io.Reader }

func (s *readerStream) Read(ctx context.Context, data []byte) (int, error) {
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	return s.reader.Read(data)
}
func (*readerStream) Close(context.Context) error { return nil }
