package runner

import (
	"bytes"
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"testing"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/programtest"
	"github.com/wspl/demi/internal/runner/process"
	"github.com/wspl/demi/internal/runnerproto"
	"go.uber.org/goleak"
)

type programTests struct{ m *testing.M }

func (p programTests) Run() int {
	return programtest.Run(p.m)
}

func TestMain(m *testing.M) {
	if handled, err := process.RunChildBootstrap(); handled {
		os.Exit(exitCode(0, err))
	}
	goleak.VerifyTestMain(programTests{m})
}

// runnerFixture drives a built runner through the same socket a backend uses.
// Every scenario owns its processes and local ports; go test -timeout guards hangs.
type runnerFixture struct {
	t                         *testing.T
	ctx                       context.Context
	cancel                    context.CancelFunc
	server                    *httptest.Server
	accepted                  chan *websocket.Conn
	requested                 chan string
	socket                    *websocket.Conn
	binary, home, state, path string
	env                       map[string]string
	command                   *exec.Cmd
	done                      chan error
	output                    runnerDiagnostics
	hello                     *runnerproto.Hello
}

type runnerDiagnostics struct {
	mu sync.Mutex
	bytes.Buffer
}

func (b *runnerDiagnostics) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.Buffer.Write(p)
}

func (b *runnerDiagnostics) text() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.String()
}

func newRunner(t *testing.T, env map[string]string, path string) *runnerFixture {
	t.Helper()
	ctx, cancel := context.WithCancel(t.Context())
	f := &runnerFixture{
		t:         t,
		ctx:       ctx,
		cancel:    cancel,
		accepted:  make(chan *websocket.Conn, 8),
		requested: make(chan string, 8),
		home:      t.TempDir(),
		state:     t.TempDir(),
		env:       env,
		path:      path,
	}
	var err error
	f.binary, err = programtest.Path(ctx, program)
	if err != nil {
		cancel()
		t.Fatal(err)
	}
	f.server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		socket, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		socket.SetReadLimit(runnerproto.MaxMessageBytes)
		select {
		case f.requested <- r.URL.RequestURI():
		case <-ctx.Done():
			_ = socket.CloseNow()
			return
		}
		select {
		case f.accepted <- socket:
		case <-ctx.Done():
			_ = socket.CloseNow()
		}
	}))
	t.Cleanup(func() {
		f.stop()
		cancel()
		f.server.Close()
		for {
			select {
			case socket := <-f.accepted:
				_ = socket.CloseNow()
			default:
				return
			}
		}
	})
	if err := os.WriteFile(filepath.Join(f.state, "runner-token"), []byte("test-token\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	f.start()
	return f
}

func (f *runnerFixture) start() {
	f.t.Helper()
	command := exec.CommandContext(f.ctx, f.binary, "run", "--backend", f.server.URL+f.path)
	command.Env = append(
		os.Environ(),
		"HOME="+f.home,
		"USERPROFILE="+f.home,
		"DEMI_HOME="+f.state,
		"DEMI_RELEASE_ID=test-release",
		"DEMI_RUNNER_MANAGED=",
		"TMPDIR=/tmp",
	)
	for name, value := range f.env {
		command.Env = append(command.Env, name+"="+value)
	}
	command.Dir = f.home
	command.Stdout = &f.output
	command.Stderr = &f.output
	if err := command.Start(); err != nil {
		f.t.Fatal(err)
	}
	f.command = command
	f.done = make(chan error, 1)
	go func() {
		f.done <- command.Wait()
	}()
	f.accept()
}

func (f *runnerFixture) accept() {
	f.t.Helper()
	select {
	case f.socket = <-f.accepted:
	case err := <-f.done:
		f.command = nil
		f.t.Fatalf("runner exited: %v\n%s", err, f.output.text())
	case <-f.ctx.Done():
		f.t.Fatalf("runner did not connect: %v\n%s", f.ctx.Err(), f.output.text())
	}
	hello, ok := f.frame().(*runnerproto.Hello)
	if !ok {
		f.t.Fatal("runner sent no hello")
	}
	f.hello = hello
}

func (f *runnerFixture) online() {
	f.send(&runnerproto.HelloOK{DeviceID: "device"})
}

func (f *runnerFixture) stop() {
	if f.command == nil {
		return
	}
	if runtime.GOOS == "windows" {
		_ = f.command.Process.Kill()
	} else {
		_ = f.command.Process.Signal(os.Interrupt)
	}
	if f.socket != nil {
		_ = f.socket.CloseNow()
		f.socket = nil
	}
	<-f.done
	f.command = nil
}

func (f *runnerFixture) send(message runnerproto.Inbound) {
	f.t.Helper()
	data, err := runnerproto.Encode(message)
	if err != nil {
		f.t.Fatal(err)
	}
	if err = f.socket.Write(f.ctx, websocket.MessageBinary, data); err != nil {
		f.t.Fatalf("send: %v\n%s", err, f.output.text())
	}
}

func (f *runnerFixture) frame() runnerproto.Outbound {
	f.t.Helper()
	_, data, err := f.socket.Read(f.ctx)
	if err != nil {
		f.t.Fatalf("receive: %v\n%s", err, f.output.text())
	}
	message, err := runnerproto.DecodeOutbound(data)
	if err != nil {
		f.t.Fatal(err)
	}
	return message
}

func runnerCommandContext() commandproto.Context {
	return commandproto.Context{
		Conversation: "conversation",
		Caller:       &commandproto.AgentCaller{Number: 1},
		Locale:       commandproto.CommandLocale{TimeZone: "UTC", Languages: []commandproto.LanguageTag{"en-US"}},
	}
}

func (f *runnerFixture) job(id, script string) {
	f.send(
		&runnerproto.JobStart{
			JobID:   id,
			Context: runnerCommandContext(),
			Script:  script,
			CWD:     f.home,
			Env:     map[string]string{},
		},
	)
}

func (f *runnerFixture) jobOutput(id string) (string, string, *runnerproto.JobExit) {
	f.t.Helper()
	var out, stderr strings.Builder
	for {
		switch message := any(f.frame()).(type) {
		case *runnerproto.JobOutput:
			if message.JobID != id {
				f.t.Fatalf("output of %s, wanted %s", message.JobID, id)
			}
			if message.Stream == runnerproto.Stdout {
				out.Write(message.Bytes)
			} else {
				stderr.Write(message.Bytes)
			}
		case *runnerproto.JobExit:
			if message.JobID != id {
				f.t.Fatalf("exit of %s, wanted %s", message.JobID, id)
			}
			return out.String(), stderr.String(), message
		default:
			f.t.Fatalf("unexpected frame %T", message)
		}
	}
}

func (f *runnerFixture) management(ctx context.Context, action string, args ...string) ([]byte, error) {
	command := exec.CommandContext(ctx, f.binary, append([]string{action, "--home", f.state}, args...)...)
	command.Env = append(os.Environ(), "DEMI_RELEASE_ID=test-release")
	return command.CombinedOutput()
}

func requireJobSuccess(t *testing.T, exit *runnerproto.JobExit, stderr string) {
	t.Helper()
	if exit.ExitCode == nil || *exit.ExitCode != 0 {
		t.Fatalf("job failed: %+v, stderr=%s", exit, stderr)
	}
}
