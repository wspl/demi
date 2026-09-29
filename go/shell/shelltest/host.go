package shelltest

import (
	"errors"
	"io"
	"path"
	"reflect"
	"strings"
	"testing"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/shell"
)

// HostConformance checks the shared Host behavior against a fresh writable
// default directory. The process scenarios use the named PATH's system tools.
// It creates only small files and waits for process and stream events.
func HostConformance(t *testing.T, host shell.Host, root, executablePath string) {
	t.Helper()
	c := hostCase{host: host, root: root, env: shell.SpawnEnv{Values: map[string]*string{"PATH": &executablePath}}}
	for _, scenario := range []struct {
		name string
		run  func(*testing.T, hostCase)
	}{
		{"identity", hostIdentity},
		{"process streams and environment", hostProcesses},
		{"filesystem namespace and metadata", hostFiles},
		{"stream ranges and atomic writes", hostStreams},
	} {
		t.Run(scenario.name, func(t *testing.T) { scenario.run(t, c) })
	}
}

type hostCase struct {
	host shell.Host
	root string
	env  shell.SpawnEnv
}

func (c hostCase) spawn(t *testing.T, command string, args []string, cwd *string) *shell.Process {
	t.Helper()
	process, err := c.host.Process().Spawn(t.Context(), shell.SpawnRequest{Command: command, Args: args, Cwd: cwd, Env: c.env})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := process.Close(); err != nil {
			t.Error(err)
		}
	})
	return process
}
func (c hostCase) run(t *testing.T, command string, args []string, cwd *string) (string, string, shell.ProcessEnd) {
	t.Helper()
	return collectProcess(t, c.spawn(t, command, args, cwd))
}
func collectProcess(t *testing.T, p *shell.Process) (string, string, shell.ProcessEnd) {
	t.Helper()
	var stdout, stderr strings.Builder
	for {
		chunk, err := p.Output.Next(t.Context())
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			t.Fatal(err)
		}
		if chunk.Stream == core.StreamKindStdout {
			stdout.Write(chunk.Bytes)
		} else {
			stderr.Write(chunk.Bytes)
		}
	}
	end, err := p.Wait(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	return stdout.String(), stderr.String(), end
}
func hostIdentity(t *testing.T, c hostCase) {
	if !path.IsAbs(c.host.DefaultCwd()) || c.host.DefaultCwd() != c.root || c.host.Identity().Hostname == "" {
		t.Fatalf("Host identity/default cwd: %+v %q", c.host.Identity(), c.host.DefaultCwd())
	}
}
func hostProcesses(t *testing.T, c hostCase) {
	out, stderr, end := c.run(t, "sh", []string{"-c", "echo out; echo err >&2; exit 3"}, nil)
	if out != "out\n" || stderr != "err\n" || end.Kind != shell.ProcessExited || end.ExitCode != 3 {
		t.Fatalf("process streams: %q %q %+v", out, stderr, end)
	}
	p := c.spawn(t, "cat", nil, nil)
	if err := p.Control.WriteStdin(t.Context(), []byte("from stdin\n")); err != nil {
		t.Fatal(err)
	}
	if err := p.Control.CloseStdin(t.Context()); err != nil {
		t.Fatal(err)
	}
	out, _, end = collectProcess(t, p)
	if out != "from stdin\n" || end.Kind != shell.ProcessExited || end.ExitCode != 0 {
		t.Fatalf("stdin: %q %+v", out, end)
	}
	out, _, end = c.run(t, "printenv", []string{"HOME"}, nil)
	if out != "" || end.Kind != shell.ProcessExited || end.ExitCode != 1 {
		t.Fatalf("exact environment: %q %+v", out, end)
	}
	// The ready line precedes the kill, so startup load cannot change the signal
	// observed. exec keeps the same PID and never exits without a signal.
	p = c.spawn(t, "sh", []string{"-c", "echo ready; exec cat"}, nil)
	var ready strings.Builder
	for !strings.Contains(ready.String(), "ready\n") {
		chunk, err := p.Output.Next(t.Context())
		if err != nil {
			t.Fatalf("readiness: %s: %v", ready.String(), err)
		}
		if chunk.Stream == core.StreamKindStdout {
			ready.Write(chunk.Bytes)
		}
	}
	if err := p.Control.Kill(t.Context(), shell.SignalTerminate); err != nil {
		t.Fatal(err)
	}
	_, _, end = collectProcess(t, p)
	if end.Kind != shell.ProcessSignalled || end.Signal != "SIGTERM" {
		t.Fatalf("terminate: %+v", end)
	}
	_, _, end = c.run(t, "definitely-not-a-host-binary", nil, nil)
	if end.Kind != shell.ProcessNotStarted || end.Error == nil || end.Error.Kind != shell.SpawnExecutableNotFound {
		t.Fatalf("missing executable: %+v", end)
	}
	missing := path.Join(c.root, "never-created")
	_, _, end = c.run(t, "echo", []string{"ok"}, &missing)
	if end.Kind != shell.ProcessNotStarted || end.Error == nil || end.Error.Kind != shell.SpawnCwdUnusable {
		t.Fatalf("missing cwd: %+v", end)
	}
}
func hostFiles(t *testing.T, c hostCase) {
	ctx := t.Context()
	fs := c.host.FS()
	if err := fs.Mkdir(ctx, "basic/src", shell.MkdirOptions{Recursive: true}); err != nil {
		t.Fatal(err)
	}
	for _, value := range []string{"first\n", "hello\ntail\n"} {
		if err := fs.WriteFile(ctx, "basic/src/file.txt", io.NopCloser(strings.NewReader(value)), shell.WriteOptions{}); err != nil {
			t.Fatal(err)
		}
	}
	file := path.Join(c.root, "basic/src/file.txt")
	contents, err := fs.ReadFile(ctx, file)
	if err != nil || string(contents) != "hello\ntail\n" {
		t.Fatalf("replace/read: %q %v", contents, err)
	}
	entries, err := fs.ReadDir(ctx, "basic/src")
	if err != nil || !reflect.DeepEqual(entries, []shell.DirEntry{{Name: "file.txt", Kind: shell.FileRegular}}) {
		t.Fatalf("listing: %+v %v", entries, err)
	}
	stat, err := fs.Stat(ctx, file)
	if err != nil || stat.Size != 11 || stat.Kind != shell.FileRegular || stat.Modified.Millisecond() <= 0 {
		t.Fatalf("stat: %+v %v", stat, err)
	}
	if err := fs.WriteFile(ctx, "basic/new/dir/g.txt", io.NopCloser(strings.NewReader("y")), shell.WriteOptions{CreateParents: true}); err != nil {
		t.Fatal(err)
	}
	if err := fs.Cp(ctx, "basic", "copied", shell.CpOptions{Recursive: true}); err != nil {
		t.Fatal(err)
	}
	if err := fs.Mv(ctx, "copied", "moved"); err != nil {
		t.Fatal(err)
	}
	if exists, err := fs.Exists(ctx, "copied"); err != nil || exists {
		t.Fatalf("move source: %t %v", exists, err)
	}
	contents, err = fs.ReadFile(ctx, "moved/new/dir/g.txt")
	if err != nil || string(contents) != "y" {
		t.Fatalf("copied descendants: %q %v", contents, err)
	}
	if err := fs.Symlink(ctx, "file.txt", "basic/src/link"); err != nil {
		t.Fatal(err)
	}
	if link, err := fs.Readlink(ctx, "basic/src/link"); err != nil || link != "file.txt" {
		t.Fatalf("readlink: %q %v", link, err)
	}
	stat, err = fs.Lstat(ctx, "basic/src/link")
	if err != nil || stat.Kind != shell.FileSymlink {
		t.Fatalf("lstat: %+v %v", stat, err)
	}
	stat, err = fs.Stat(ctx, "basic/src/link")
	if err != nil || stat.Kind != shell.FileRegular {
		t.Fatalf("stat follows: %+v %v", stat, err)
	}
	if real, err := fs.Realpath(ctx, "basic/src/link"); err != nil || real != file {
		t.Fatalf("realpath: %q %v", real, err)
	}
	if err := fs.Link(ctx, file, "basic/src/hard"); err != nil {
		t.Fatal(err)
	}
	if err := fs.Chmod(ctx, file, 0600); err != nil {
		t.Fatal(err)
	}
	when, err := core.TimestampFromMillisecond(1600000000000)
	if err != nil {
		t.Fatal(err)
	}
	if err := fs.Utimes(ctx, file, when, when); err != nil {
		t.Fatal(err)
	}
	stat, err = fs.Stat(ctx, "basic/src/hard")
	if err != nil || stat.Mode&0777 != 0600 || stat.Modified != when {
		t.Fatalf("hard link metadata: %+v %v", stat, err)
	}
	cwd := path.Join(c.root, "basic/src")
	out, _, end := c.run(t, "sh", []string{"-c", "cat file.txt; printf back > written.txt"}, &cwd)
	if out != "hello\ntail\n" || end.Kind != shell.ProcessExited || end.ExitCode != 0 {
		t.Fatalf("shared process namespace: %q %+v", out, end)
	}
	contents, err = fs.ReadFile(ctx, "basic/src/written.txt")
	if err != nil || string(contents) != "back" {
		t.Fatalf("shared fs namespace: %q %v", contents, err)
	}
	if err := fs.Rm(ctx, "moved", shell.RmOptions{Recursive: true}); err != nil {
		t.Fatal(err)
	}
	if exists, err := fs.Exists(ctx, "moved"); err != nil || exists {
		t.Fatalf("recursive remove: %t %v", exists, err)
	}
	if err := fs.Rm(ctx, "absent", shell.RmOptions{Force: true}); err != nil {
		t.Fatal(err)
	}
	for name, operation := range map[string]func() error{
		"read": func() error {
			_, err := fs.ReadFile(ctx, "absent")
			return err
		},
		"stat": func() error {
			_, err := fs.Stat(ctx, "absent")
			return err
		},
		"readdir": func() error {
			_, err := fs.ReadDir(ctx, "absent")
			return err
		},
		"rm": func() error { return fs.Rm(ctx, "absent", shell.RmOptions{}) },
	} {
		var hostError *shell.HostError
		if err := operation(); !errors.As(err, &hostError) || hostError.Code != "ENOENT" {
			t.Fatalf("%s: %v", name, err)
		}
	}
	var hostError *shell.HostError
	if err := fs.Mkdir(ctx, "basic", shell.MkdirOptions{}); !errors.As(err, &hostError) || hostError.Code != "EEXIST" {
		t.Fatalf("existing directory: %v", err)
	}
}

type brokenContents struct {
	sent bool
	err  error
}

func (b *brokenContents) Read(data []byte) (int, error) {
	if b.sent {
		return 0, b.err
	}
	b.sent = true
	return copy(data, "partial"), nil
}
func (*brokenContents) Close() error { return nil }
func hostStreams(t *testing.T, c hostCase) {
	fs := c.host.FS()
	ctx := t.Context()
	if err := fs.WriteFile(ctx, "stream/digits.txt", io.NopCloser(strings.NewReader("0123456789")), shell.WriteOptions{CreateParents: true}); err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		span shell.ByteRange
		want string
	}{
		{shell.ByteRange{}, "0123456789"}, {shell.ByteRange{Offset: 3, Length: new(uint64(4))}, "3456"}, {shell.ByteRange{Offset: 7}, "789"},
	} {
		stream, err := fs.ReadStream(ctx, "stream/digits.txt", tc.span)
		if err != nil {
			t.Fatal(err)
		}
		data, err := io.ReadAll(stream)
		closeErr := stream.Close()
		if err != nil || closeErr != nil || string(data) != tc.want {
			t.Fatalf("range: %q %v %v", data, err, closeErr)
		}
	}
	stream, err := fs.ReadStream(ctx, "stream/missing", shell.ByteRange{})
	if stream != nil {
		stream.Close()
	}
	var hostError *shell.HostError
	if !errors.As(err, &hostError) || hostError.Code != "ENOENT" {
		t.Fatalf("open refusal before bytes: %v", err)
	}
	failure := &shell.HostError{Kind: shell.HostInterrupted, Message: "the stream broke"}
	err = fs.WriteFile(ctx, "stream/digits.txt", &brokenContents{err: failure}, shell.WriteOptions{})
	if !errors.Is(err, failure) {
		t.Fatalf("source error preserved: %v", err)
	}
	contents, err := fs.ReadFile(ctx, "stream/digits.txt")
	if err != nil || string(contents) != "0123456789" {
		t.Fatalf("failed write replaced original: %q %v", contents, err)
	}
	entries, err := fs.ReadDir(ctx, "stream")
	if err != nil || len(entries) != 1 || entries[0].Name != "digits.txt" {
		t.Fatalf("partial file left behind: %+v %v", entries, err)
	}
}
