//go:build darwin || linux

package runner

import (
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/programtest"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runner/cmdpkgs/cmdpkgstest"
	"github.com/wspl/demi/internal/runner/host"
	"github.com/wspl/demi/internal/runner/jobs/jobstest"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runner/shell/shelltest"
	"github.com/wspl/demi/internal/runnerwire"
	"golang.org/x/sys/unix"
)

// Descriptor exhaustion changes process-wide state, so this scenario runs in
// an isolated copy of the test executable. About two seconds plus its native
// fixture build: it covers the composed owners under actual EMFILE, not a mock.
func TestRunningOutOfOpenFilesWaitsInsteadOfFailing(t *testing.T) {
	if os.Getenv("DEMI_TEST_RUNNER_OPEN_FILES") != "1" {
		native, err := programtest.Path(t.Context(), "demi-native-fixture")
		if err != nil {
			t.Fatal(err)
		}
		executable, err := os.Executable()
		if err != nil {
			t.Fatal(err)
		}
		ctx, cancel := context.WithTimeout(t.Context(), 60*time.Second)
		defer cancel()
		command := exec.CommandContext(ctx, "/bin/sh", "-c", `ulimit -Sn 1024 && exec "$1" -test.run='^TestRunningOutOfOpenFilesWaitsInsteadOfFailing$' -test.count=1 -test.timeout=55s`, "sh", executable)
		command.Env = append(os.Environ(), "DEMI_TEST_RUNNER_OPEN_FILES=1", "DEMI_TEST_NATIVE="+native, "TMPDIR=/tmp")
		if output, err := command.CombinedOutput(); err != nil {
			t.Fatalf("descriptor subprocess: %v\n%s", err, output)
		}
		return
	}
	ctx, cancel := context.WithTimeout(t.Context(), 45*time.Second)
	defer cancel()
	started, raised, err := process.RaiseOpenFileLimit()
	if err != nil || started != 1024 || raised <= started {
		t.Fatalf("startup limit %d/%d: %v", started, raised, err)
	}
	var original unix.Rlimit
	if err := unix.Getrlimit(unix.RLIMIT_NOFILE, &original); err != nil {
		t.Fatal(err)
	}
	root := t.TempDir()
	scope := shelltest.NewScope(ctx, nil)
	defer func() {
		scope.Cancel()
		scope.Finish(context.Background())
	}()
	// Check launch inheritance before the exhaustion fixture changes process limits.
	child, err := process.Spawn(ctx, process.SpawnOptions{Command: "/bin/sh", Args: []string{"-c", "ulimit -Sn"}, Cwd: root, Env: map[string]string{}, ProcessGroup: true})
	if err != nil {
		t.Fatal(err)
	}
	var printed strings.Builder
	for chunk := range child.Output {
		if chunk.Stream == runnerwire.Stdout {
			printed.Write(chunk.Bytes)
		}
	}
	result := child.Wait(ctx)
	if result.Code == nil || *result.Code != 0 || printed.String() != "1024\n" {
		t.Fatalf("child launch limit: %+v, %q", result, printed.String())
	}
	limits, err := scope.Start(ctx, "/bin/sh -c 'ulimit -Sn'; env /bin/sh -c 'ulimit -Sn'", root, map[string]string{"HOME": root, "PATH": "/usr/bin:/bin"}, false)
	if err != nil {
		t.Fatal(err)
	}
	printed.Reset()
	for chunk := range limits.Output() {
		if chunk.Stream == runnerwire.Stdout {
			printed.Write(chunk.Bytes)
		}
	}
	result, _ = limits.Wait(ctx)
	if result.Code == nil || *result.Code != 0 || printed.String() != "1024\n1024\n" {
		t.Fatalf("shell launch limits: %+v, %q", result, printed.String())
	}
	limited := original
	limited.Cur = 1024
	if err := unix.Setrlimit(unix.RLIMIT_NOFILE, &limited); err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := unix.Setrlimit(unix.RLIMIT_NOFILE, &original); err != nil {
			t.Error(err)
		}
	}()
	// All network readers use connection lifetimes; Close joins them after tests.
	backend := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodPut {
			_, err := io.Copy(io.Discard, r.Body)
			if err == nil {
				w.WriteHeader(http.StatusOK)
			}
			return
		}
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("hello"))
		w.(http.Flusher).Flush()
		<-r.Context().Done()
	}))
	defer backend.Close()
	backendURL, err := runnerwire.ParseBackendURL(backend.URL)
	if err != nil {
		t.Fatal(err)
	}
	pipes, err := process.NewPipeClient(backendURL, func() (runnerwire.DeviceToken, bool) { return "token", true })
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = pipes.Close() }()
	exhaustDescriptors(ctx, t, "pipe download", func() error {
		stream, err := pipes.Open(ctx, "/pipe/download")
		if err != nil {
			return err
		}
		defer func() { _ = stream.Close() }()
		bytes := make([]byte, 5)
		_, err = io.ReadFull(stream, bytes)
		if string(bytes) != "hello" {
			return fmt.Errorf("pipe bytes %q", bytes)
		}
		return err
	})
	exhaustDescriptors(ctx, t, "pipe upload", func() error { return pipes.Put(ctx, "/pipe/upload", io.NopCloser(strings.NewReader("hello"))) })
	output := make(chan []byte, 64)
	service := host.New(ctx, root, pipes, output)
	defer func() {
		if err := service.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	exhaustDescriptors(ctx, t, "filesystem", func() error {
		if err := service.Readdir(ctx, runnerwire.FSReaddir{ID: "directory", Path: root}); err != nil {
			return err
		}
		return expectHostReply(ctx, output, "directory")
	})
	file := filepath.Join(root, "transfer")
	if err := os.WriteFile(file, []byte("content"), 0600); err != nil {
		t.Fatal(err)
	}
	exhaustDescriptors(ctx, t, "file transfer", func() error {
		if err := service.ReadFile(ctx, runnerwire.FSReadFile{ID: "file", Path: file, Output: runnerwire.PipeRef{ID: "file-out", URL: "/pipe/upload"}}); err != nil {
			return err
		}
		return expectHostReply(ctx, output, "file")
	})
	repository := filepath.Join(root, "repository")
	if err := os.Mkdir(repository, 0700); err != nil {
		t.Fatal(err)
	}
	for _, args := range [][]string{{"init", "-q"}, {"config", "user.name", "Fixture"}, {"config", "user.email", "fixture@example.com"}} {
		command := exec.CommandContext(ctx, "git", args...)
		command.Dir = repository
		if out, err := command.CombinedOutput(); err != nil {
			t.Fatalf("git: %v %s", err, out)
		}
	}
	if err := os.WriteFile(filepath.Join(repository, "a.txt"), []byte("before\n"), 0600); err != nil {
		t.Fatal(err)
	}
	for _, args := range [][]string{{"add", "a.txt"}, {"commit", "-qm", "fixture"}} {
		command := exec.CommandContext(ctx, "git", args...)
		command.Dir = repository
		if out, err := command.CombinedOutput(); err != nil {
			t.Fatalf("git: %v %s", err, out)
		}
	}
	if err := os.WriteFile(filepath.Join(repository, "a.txt"), []byte("after\n"), 0600); err != nil {
		t.Fatal(err)
	}
	exhaustDescriptors(ctx, t, "working tree", func() error {
		if err := service.GitChanges(ctx, runnerwire.GitChangesMessage{ID: "git", Root: repository}); err != nil {
			return err
		}
		for {
			select {
			case <-ctx.Done():
				return ctx.Err()
			case bytes := <-output:
				frame, err := runnerwire.DecodeOutbound(bytes)
				if err != nil {
					return err
				}
				if reply, ok := frame.(*runnerwire.GitOK); ok && reply.ID == "git" {
					result, ok := reply.Result.(*runnerwire.GitChangesResult)
					if !ok || !result.Value.Repository || len(result.Value.Files) != 1 || result.Value.Files[0].Path != "a.txt" || result.Value.Files[0].Kind != runnerwire.ChangeKindModified {
						return fmt.Errorf("working tree changed under exhaustion: %+v", reply)
					}
					return nil
				}
				if reply, ok := frame.(*runnerwire.GitError); ok {
					return fmt.Errorf("working tree: %s", reply.Message)
				}
			}
		}
	})
	socket, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	networkDone := make(chan struct{})
	go func() {
		defer close(networkDone)
		conn, err := socket.Accept()
		if err == nil {
			defer func() { _ = conn.Close() }()
			<-ctx.Done()
		}
	}()
	defer func() {
		cancel()
		_ = socket.Close()
		<-networkDone
	}()
	exhaustDescriptors(ctx, t, "network stream", func() error {
		operation, stop := context.WithCancel(ctx)
		done := make(chan error, 1)
		go func() {
			done <- service.NetOpen(operation, runnerwire.NetOpen{StreamID: "network", Host: "127.0.0.1", Port: uint16(socket.Addr().(*net.TCPAddr).Port), Input: runnerwire.PipeRef{ID: "net-in", URL: "/pipe/download"}, Output: runnerwire.PipeRef{ID: "net-out", URL: "/pipe/upload"}})
		}()
		defer func() {
			stop()
			<-done
		}()
		for {
			select {
			case <-ctx.Done():
				return ctx.Err()
			case bytes := <-output:
				frame, err := runnerwire.DecodeOutbound(bytes)
				if err != nil {
					return err
				}
				if reply, ok := frame.(*runnerwire.NetOpened); ok && reply.StreamID == "network" {
					return nil
				}
				if reply, ok := frame.(*runnerwire.NetError); ok {
					return fmt.Errorf("network: %s", reply.Message)
				}
			}
		}
	})
	exhaustDescriptors(ctx, t, "process", func() error {
		child, err := process.Spawn(ctx, process.SpawnOptions{Command: "/bin/echo", Args: []string{"hello"}, Cwd: root, Env: map[string]string{}, ProcessGroup: true})
		if err != nil {
			return err
		}
		defer child.Cancel()
		for range child.Output {
		}
		exit := child.Wait(ctx)
		if exit.Code == nil || *exit.Code != 0 {
			return fmt.Errorf("process: %+v", exit)
		}
		return nil
	})
	exhaustDescriptors(ctx, t, "shell job start", func() error {
		job, err := scope.Start(ctx, "true", root, map[string]string{"HOME": root, "PATH": "/usr/bin:/bin"}, false)
		if err != nil {
			return err
		}
		defer job.Cancel()
		for range job.Output() {
		}
		exit, _ := job.Wait(ctx)
		if exit.Code == nil || *exit.Code != 0 {
			return fmt.Errorf("job: %+v", exit)
		}
		return nil
	})

	registry, err := cmdpkgs.NewServiceRegistry(ctx, filepath.Join(root, "cache"), "", root, map[string]string{})
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := registry.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	native := os.Getenv("DEMI_TEST_NATIVE")
	bytes, err := os.ReadFile(native)
	if err != nil {
		t.Fatal(err)
	}
	target, err := commandwire.HostTarget()
	if err != nil {
		t.Fatal(err)
	}
	descriptor := commandwire.PackageDescriptor{ID: "fixture", Version: "1.0.0", ProtocolVersion: 1, Operations: (&cmdpkgstest.Fixture{}).Operations(), Targets: map[string]commandwire.PackageArtifact{string(target): {SHA256: fmt.Sprintf("%x", sha256.Sum256(bytes)), Size: uint64(len(bytes))}}, Resources: map[string]commandwire.PackageResource{}}
	exhaustDescriptors(ctx, t, "native service start", func() error {
		resident, err := registry.Handle().Acquire(ctx, descriptor, localFixtureArtifact(native), cmdpkgstest.NoNumbers{})
		if err != nil {
			return err
		}
		_, err = resident.Client().Info(ctx)
		return err
	})
	manifest, err := os.ReadFile("testdata/declared-help.json")
	if err != nil {
		t.Fatal(err)
	}
	dispatch := jobstest.NewDispatch(ctx, t, filepath.Join(root, "dispatch"), manifest, pipes)
	execution, guard, err := dispatch.Context(ctx, "local", runnerCommandContext())
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := guard.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	raw, err := process.NewRawCommand(execution.ID, "fixture", []string{"--help"}, false)
	if err != nil {
		t.Fatal(err)
	}
	args, err := raw.MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	exhaustDescriptors(ctx, t, "local command connection", func() error {
		completion, err := process.Forward(ctx, dispatch.Server.Endpoint(), commandwire.LocalInvocation{Operation: process.Raw, InvocationID: "local", Args: args, Cwd: root, Env: map[string]string{}}, process.Stdio{Stdin: io.NopCloser(strings.NewReader("")), Stdout: discardCommandOutput{}, Stderr: discardCommandOutput{}})
		if err != nil {
			return err
		}
		if completion.ExitCode != 0 {
			return fmt.Errorf("command exit %d", completion.ExitCode)
		}
		return nil
	})
	if err := dispatch.Close(context.Background()); err != nil {
		t.Fatal(err)
	}
	t.Run("utility_child", func(t *testing.T) {
		// Unlike Rust's embedded xargs, Go starts the system utility in its own
		// process. It can start its child while the runner has no descriptor left.
		job, err := scope.Start(ctx, "sh -c 'printf ready; xargs /usr/bin/printf'", root, map[string]string{"HOME": root, "PATH": "/usr/bin:/bin"}, false)
		if err != nil {
			t.Fatal(err)
		}
		defer job.Cancel()
		awaitReady(ctx, t, scope, job)
		held := fillDescriptors(t)
		defer releaseDescriptors(held)
		job.Input() <- process.Input{Bytes: []byte("three\n")}
		job.Input() <- process.Input{}
		var output strings.Builder
		for chunk := range job.Output() {
			output.Write(chunk.Bytes)
		}
		exit, _ := job.Wait(ctx)
		if exit.Code == nil || *exit.Code != 0 || output.String() != "three" {
			t.Fatalf("utility child: %+v, %q", exit, output.String())
		}
	})
	t.Run("running_shell", func(t *testing.T) {
		job, err := scope.Start(ctx, "printf ready; read go; echo one | cat > piped.txt; cat <<EOF 2>&1 > here.txt\ntwo\nEOF\n", root, map[string]string{"HOME": root, "PATH": "/usr/bin:/bin"}, false)
		if err != nil {
			t.Fatal(err)
		}
		awaitReady(ctx, t, scope, job)
		exhaustDescriptors(ctx, t, "running pipeline and redirections", func() error {
			job.Input() <- process.Input{Bytes: []byte("go\n")}
			job.Input() <- process.Input{}
			for range job.Output() {
			}
			exit, _ := job.Wait(ctx)
			if exit.Error != nil {
				return errors.New(*exit.Error)
			}
			if exit.Code == nil || *exit.Code != 0 {
				return fmt.Errorf("running job: %+v", exit)
			}
			return nil
		})
		for name, want := range map[string]string{"piped.txt": "one\n", "here.txt": "two\n"} {
			bytes, err := os.ReadFile(filepath.Join(root, name))
			if err != nil || string(bytes) != want {
				t.Fatalf("%s: %q, %v", name, bytes, err)
			}
		}
	})
	t.Run("cancelled_pipeline", func(t *testing.T) {
		cancelled, err := scope.Start(ctx, "printf ready; read go; echo one | cat > cancelled.txt", root, map[string]string{"HOME": root, "PATH": "/usr/bin:/bin"}, false)
		if err != nil {
			t.Fatal(err)
		}
		awaitReady(ctx, t, scope, cancelled)
		held := fillDescriptors(t)
		defer releaseDescriptors(held)
		joined := make(chan error, 1)
		var exit process.Exit
		go func() {
			defer close(joined)
			exit, _ = cancelled.Wait(ctx)
			if exit.Error != nil {
				joined <- errors.New(*exit.Error)
			} else {
				joined <- nil
			}
		}()
		defer func() {
			cancelled.Cancel()
			<-joined
		}()
		before := cmdsdk.DescriptorPauses()
		cancelled.Input() <- process.Input{Bytes: []byte("go\n")}
		waitDescriptorPause(ctx, t, before, joined)
		cancelled.Cancel()
		<-joined
		if exit.Signal == nil || *exit.Signal != "SIGKILL" {
			t.Fatalf("cancelled descriptor wait: %+v", exit)
		}

	})

}

// fillDescriptors owns every available descriptor until the starvation step releases it.
func fillDescriptors(t *testing.T) []int {
	t.Helper()
	var held []int
	for {
		fd, err := unix.Open("/dev/null", unix.O_RDONLY|unix.O_CLOEXEC, 0)
		if errors.Is(err, unix.EMFILE) {
			return held
		}
		if err != nil {
			releaseDescriptors(held)
			t.Fatal(err)
		}
		held = append(held, fd)
	}
}
func releaseDescriptors(held []int) {
	for _, fd := range held {
		_ = unix.Close(fd)
	}
} // Only this scope owns these descriptors.
func waitDescriptorPause(ctx context.Context, t *testing.T, before uint64, done <-chan error) {
	t.Helper()
	for cmdsdk.DescriptorPauses() == before {
		select {
		case err := <-done:
			t.Fatalf("operation ended instead of waiting for a descriptor: %v", err)
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		default:
			runtime.Gosched()
		}
	}
}
func exhaustDescriptors(ctx context.Context, t *testing.T, name string, operation func() error) {
	t.Helper()
	t.Log(name)
	held := fillDescriptors(t)
	before := cmdsdk.DescriptorPauses()
	done := make(chan error, 1)
	go func() {
		defer close(done)
		done <- operation()
	}()
	defer func() {
		releaseDescriptors(held)
		<-done
	}()
	waitDescriptorPause(ctx, t, before, done)
	select {
	case err := <-done:
		t.Fatalf("%s completed while exhausted: %v", name, err)
	default:
	}
	releaseDescriptors(held)
	held = nil
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
}
func expectHostReply(ctx context.Context, output <-chan []byte, id string) error {
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case data := <-output:
			message, err := runnerwire.DecodeOutbound(data)
			if err != nil {
				return err
			}
			if reply, ok := message.(*runnerwire.FSOK); ok && reply.ID == id {
				return nil
			}
			if reply, ok := message.(*runnerwire.FSError); ok && reply.ID == id {
				return errors.New(reply.Message)
			}
		}
	}
}
func awaitReady(ctx context.Context, t *testing.T, scope *shelltest.Scope, job process.ShellJob) {
	t.Helper()
	var printed strings.Builder
	for !strings.HasSuffix(printed.String(), "ready") {
		select {
		case chunk, ok := <-job.Output():
			if !ok {
				t.Fatal("job ended before ready")
			}
			if chunk.Stream == runnerwire.Stdout {
				printed.Write(chunk.Bytes)
			}
		case <-ctx.Done():
			t.Fatal(ctx.Err())
		}
	}
	if err := scope.Activity().WaitWaiting(ctx); err != nil {
		t.Fatal(err)
	}
}

type localFixtureArtifact string

func (p localFixtureArtifact) Resolve(context.Context, commandwire.PackageArtifact) (cmdpkgs.ArtifactSource, error) {
	return cmdpkgs.ArtifactSource{Path: string(p)}, nil
}

type discardCommandOutput struct{}

func (discardCommandOutput) Write(p []byte) (int, error) { return len(p), nil }
func (discardCommandOutput) Close() error                { return nil }
