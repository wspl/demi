package backendtest

import (
	"errors"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest/controlproto"
	"github.com/wspl/demi/go/backendtest/procgroup"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// MasterEmail and MasterPassword are the master account a scenario sets up.
const (
	MasterEmail    = "master@example.test"
	MasterPassword = "master-pass-1"
)

// SessionCookie is the name of the session cookie.
const SessionCookie = "demi_session"

// Ptr returns a pointer to value, for the optional members of the tuning file.
func Ptr[T any](value T) *T {
	return &value
}

// A Harness is what a scenario's backends start from: a data directory, the
// machine manager the scenario scripts, the native packages the backends
// publish, and the tuning file. It can start a backend several times over the
// same data directory; a later start listens on the port of the first, so the
// runners that paired with the first find it again.
type Harness struct {
	t testing.TB
	// Root holds everything of the scenario: the data directory, the tuning
	// and native configuration, and the scripted manager's and runners' files.
	Root string
	// Manager runs the Clouds of every backend this harness starts.
	Manager *scripted.Manager
	// Tuning is written to the tuning file every start reads. The scenario
	// changes it before it starts a backend.
	Tuning controlproto.Tuning

	mode           string
	exposeDomain   string
	webDirectory   string
	runnerReleases string
	claudeReleases string
	packages       []Package
	port           int
	running        *Backend
}

// An Option sets what a harness starts its backends with.
type Option func(*Harness)

// WithMode sets the instance mode: "shared" or "isolated".
func WithMode(mode string) Option {
	return func(h *Harness) { h.mode = mode }
}

// WithExposeDomain sets the domain expose hostnames live under.
func WithExposeDomain(domain string) Option {
	return func(h *Harness) { h.exposeDomain = domain }
}

// WithPackage publishes a native package the workspace built, whose program
// every runner installs from the backend.
func WithPackage(pkg Package) Option {
	return func(h *Harness) { h.packages = append(h.packages, pkg) }
}

// WithBuiltin makes conversations' commands bind to the demi.builtin package.
func WithBuiltin() Option { return WithPackage(BuiltinPackage) }

// WithClaudePackage publishes the demi.claude package, which lists and
// installs a Claude Code provider's CLI.
func WithClaudePackage() Option { return WithPackage(ClaudePackage) }

// WithNativeFixture makes conversations' user streams bind to the runner's
// native test fixture (FixtureStreams).
func WithNativeFixture() Option {
	return func(h *Harness) {
		h.packages = append(h.packages, FixturePackage)
		streams := map[string]controlproto.NativeOperation{}
		for _, name := range FixtureStreams {
			streams[name] = controlproto.NativeOperation{Package: FixturePackage.ID, Operation: name}
		}
		h.Tuning.UserStreams = &streams
	}
}

// WithClockAt starts the manual clock at at instead of the suite's shared
// start, for a scenario that compares the backend's times with those the file
// system gives the objects it writes.
func WithClockAt(at time.Time) Option {
	return func(h *Harness) { h.Tuning.ClockStartMs = Ptr(at.UnixMilli()) }
}

// WithCodex points the codex family at a scripted Codex, both its sign-in service
// and its backend.
func WithCodex(codex *scripted.Codex) Option {
	return func(h *Harness) {
		h.Tuning.Families = &controlproto.FamiliesTuning{
			Codex: &controlproto.CodexTuning{BackendURL: codex.URL(""), AuthURL: codex.URL("")},
		}
	}
}

// WithMail gives the backend a mail sender that keeps the verification mail it
// sends, which Control.Mail lists.
func WithMail() Option {
	return func(h *Harness) { h.Tuning.Mail = Ptr(true) }
}

// WithRunnerReleases sets the runner releases the installer routes serve.
func WithRunnerReleases(directory string) Option {
	return func(h *Harness) { h.runnerReleases = directory }
}

// WithModelsDev sets where the backend reads the models.dev document.
func WithModelsDev(url string) Option {
	return func(h *Harness) { h.Tuning.ModelsDevURL = &url }
}

// WithClaudeReleases sets where the backend reads the Claude Code distribution.
func WithClaudeReleases(url string) Option {
	return func(h *Harness) { h.claudeReleases = url }
}

// WithWeb puts a browser build in the data directory's neighbour, served beside
// the API.
func WithWeb(files map[string]string) Option {
	return func(h *Harness) {
		directory := filepath.Join(h.Root, "web")
		if err := os.MkdirAll(directory, 0o755); err != nil {
			h.t.Fatal(err)
		}
		for name, content := range files {
			if err := os.WriteFile(filepath.Join(directory, name), []byte(content), 0o644); err != nil {
				h.t.Fatal(err)
			}
		}
		h.webDirectory = directory
	}
}

// New makes a harness. The backends it starts serve one shard thread, as the
// Rust suite's do, with the product's defaults but for three: no liveness ping
// (a scenario ends runners itself), no title request beside the first turn (a
// scripted vendor answers only the turns a scenario scripts), and no retention
// pass by itself (a scenario runs one when it chooses).
func New(t testing.TB, options ...Option) *Harness {
	t.Helper()
	programsDir(t)
	// A socket path is short: the limit is about a hundred bytes.
	root, err := os.MkdirTemp("", "demi-")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		// What a stopped scenario leaves is a temporary directory of ours.
		_ = os.RemoveAll(root)
	})
	// The jobs of a runner that was killed have no parent left but this
	// process, which ends them once the scenario's processes are gone.
	t.Cleanup(func() { procgroup.KillAdopted(root) })
	h := &Harness{
		t:    t,
		Root: root,
		mode: "shared",
		Tuning: controlproto.Tuning{
			Runners:       &controlproto.RunnersTuning{PingMs: Ptr(uint64(0))},
			Conversations: &controlproto.ConversationsTuning{Titles: Ptr(false)},
			Lifecycle:     &controlproto.LifecycleTuning{RetentionIntervalMs: Ptr(uint64(0))},
		},
	}
	h.Manager = scripted.StartManager(t, Program(t, "demi-runner"))
	for _, option := range options {
		option(h)
	}
	return h
}

// SetMode sets the instance mode the next start uses: "shared" or "isolated".
func (h *Harness) SetMode(mode string) {
	h.mode = mode
}

// Runners returns the tuning of runner connections, to change.
func (h *Harness) Runners() *controlproto.RunnersTuning {
	if h.Tuning.Runners == nil {
		h.Tuning.Runners = &controlproto.RunnersTuning{}
	}
	return h.Tuning.Runners
}

// Lifecycle returns the tuning of the idle window and the retention pass, to
// change.
func (h *Harness) Lifecycle() *controlproto.LifecycleTuning {
	if h.Tuning.Lifecycle == nil {
		h.Tuning.Lifecycle = &controlproto.LifecycleTuning{}
	}
	return h.Tuning.Lifecycle
}

// Cloud returns the tuning of the Cloud, to change.
func (h *Harness) Cloud() *controlproto.CloudTuning {
	if h.Tuning.Cloud == nil {
		h.Tuning.Cloud = &controlproto.CloudTuning{}
	}
	return h.Tuning.Cloud
}

// Conversations returns the tuning of conversations, to change.
func (h *Harness) Conversations() *controlproto.ConversationsTuning {
	if h.Tuning.Conversations == nil {
		h.Tuning.Conversations = &controlproto.ConversationsTuning{}
	}
	return h.Tuning.Conversations
}

// Pages returns the tuning of the sockets to a page, to change.
func (h *Harness) Pages() *controlproto.PagesTuning {
	if h.Tuning.Pages == nil {
		h.Tuning.Pages = &controlproto.PagesTuning{}
	}
	return h.Tuning.Pages
}

// Exposes returns the tuning of the public relay, to change.
func (h *Harness) Exposes() *controlproto.ExposesTuning {
	if h.Tuning.Exposes == nil {
		h.Tuning.Exposes = &controlproto.ExposesTuning{}
	}
	return h.Tuning.Exposes
}

// Logins returns the timing of device logins, to change.
func (h *Harness) Logins() *controlproto.LoginsTuning {
	if h.Tuning.Logins == nil {
		h.Tuning.Logins = &controlproto.LoginsTuning{}
	}
	return h.Tuning.Logins
}

// DataDir is the data directory the backend keeps its storage in.
func (h *Harness) DataDir() string {
	return filepath.Join(h.Root, "backend")
}

// A Backend is a running demi-backend process.
type Backend struct {
	t testing.TB
	h *Harness
	// URL is the backend's origin, such as http://127.0.0.1:41234.
	URL string
	// Port is the port it listens on.
	Port int
	// Control is the client of its control socket.
	Control *Control

	cmd    *exec.Cmd
	exited chan struct{}
	output *outputBuffer
	client *http.Client
	ended  bool
}

// Logs returns what the backend printed.
func (b *Backend) Logs() string {
	return b.output.String()
}

// Start starts a backend over the harness's data directory and returns once it
// serves. The scenario stops it with Stop; the test's cleanup kills one that is
// left.
func (h *Harness) Start() *Backend {
	h.t.Helper()
	if h.running != nil && !h.running.ended {
		h.t.Fatal("the harness's backend still runs")
	}
	// A restart listens on the port of the first start, which the last backend
	// gives back a moment after it exits; a first start takes a port a probe
	// found free, which another scenario may take meanwhile.
	for attempt := 0; ; attempt++ {
		if h.port == 0 {
			h.port = freePort(h.t)
		}
		backend, err := h.launch(h.port)
		if err == nil {
			h.running = backend
			return backend
		}
		if attempt >= 40 || !errors.Is(err, errEndedBeforeServing) {
			h.t.Fatalf("the backend did not start: %v", err)
		}
		if h.running == nil {
			// The first start's port was taken: probe for another.
			h.port = 0
		}
		time.Sleep(250 * time.Millisecond)
	}
}

// StartSetUp starts a backend with the master account set up and signed in.
func (h *Harness) StartSetUp() (*Backend, *Session) {
	h.t.Helper()
	backend := h.Start()
	return backend, backend.Setup()
}

// freePort returns a loopback port nothing listens on.
func freePort(t testing.TB) int {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	return listener.Addr().(*net.TCPAddr).Port
}

func (h *Harness) launch(port int) (*Backend, error) {
	tuning, err := controlproto.EncodeTuning(h.Tuning)
	if err != nil {
		return nil, err
	}
	tuningPath := filepath.Join(h.Root, "tuning.json")
	if err := os.WriteFile(tuningPath, tuning, 0o644); err != nil {
		return nil, err
	}
	nativePath := writeNativeConfig(h.t, h.Root, h.packages)
	controlPath := filepath.Join(h.Root, "control.sock")
	// A socket a backend killed before it stopped left behind.
	_ = os.Remove(controlPath)
	url := "http://127.0.0.1:" + strconv.Itoa(port)
	env := []string{
		"PATH=" + os.Getenv("PATH"),
		"HOME=" + h.Root,
		"DEMI_BACKEND_DATA=" + h.DataDir(),
		"DEMI_BACKEND_PORT=" + strconv.Itoa(port),
		"DEMI_INSTANCE_MODE=" + h.mode,
		"DEMI_BACKEND_PUBLIC_URL=" + url,
		"DEMI_MACHINES_SOCKET=" + h.Manager.Socket(),
		"DEMI_NATIVE_CONFIG=" + nativePath,
		controlproto.ControlVariable + "=" + controlPath,
		controlproto.TuningVariable + "=" + tuningPath,
	}
	if h.exposeDomain != "" {
		env = append(env, "DEMI_EXPOSE_DOMAIN="+h.exposeDomain)
	}
	if h.webDirectory != "" {
		env = append(env, "DEMI_WEB_DIRECTORY="+h.webDirectory)
	}
	if h.runnerReleases != "" {
		env = append(env, "DEMI_RUNNER_RELEASE_DIR="+h.runnerReleases)
	}
	// No scenario reaches the public Claude Code distribution: one that installs
	// the CLI serves it with WithClaudeReleases, and the others find a port
	// nothing listens on.
	claude := h.claudeReleases
	if claude == "" {
		claude = "http://127.0.0.1:9/claude-code-releases"
	}
	env = append(env, "DEMI_CLAUDE_RELEASES_URL="+claude)

	cmd := exec.Command(Program(h.t, "demi-backend"))
	cmd.Env = env
	cmd.Dir = h.Root
	output := &outputBuffer{}
	readOutput, writeOutput, err := os.Pipe()
	if err != nil {
		return nil, err
	}
	cmd.Stdout = writeOutput
	cmd.Stderr = writeOutput
	err = procgroup.Start(cmd)
	// The backend has its own copy of the write end.
	_ = writeOutput.Close()
	if err != nil {
		_ = readOutput.Close()
		return nil, err
	}
	exited := make(chan struct{})
	go func() {
		defer readOutput.Close()
		output.copyFrom(readOutput)
	}()
	go func() {
		// The exit status is read from the process state by whoever stops it.
		_ = cmd.Wait()
		close(exited)
	}()
	backend := &Backend{
		t:      h.t,
		h:      h,
		URL:    url,
		Port:   port,
		cmd:    cmd,
		exited: exited,
		output: output,
		client: newClient(),
	}
	h.t.Cleanup(backend.kill)
	control, err := backend.waitServing(controlPath)
	if err != nil {
		backend.kill()
		return nil, err
	}
	backend.Control = control
	return backend, nil
}

// errEndedBeforeServing is a backend that exited while the harness waited for it
// to serve, as one does whose port another process took.
var errEndedBeforeServing = errors.New("the backend ended before it served")

// waitServing waits until the control socket accepts a connection, which a test
// build opens once it serves.
func (b *Backend) waitServing(controlPath string) (*Control, error) {
	deadline := time.After(Patience)
	for {
		select {
		case <-b.exited:
			return nil, fmt.Errorf("%w:\n%s", errEndedBeforeServing, b.output.String())
		case <-deadline:
			return nil, fmt.Errorf("the backend did not serve within %s:\n%s", Patience, b.output.String())
		default:
		}
		control, err := dialControl(b.t, controlPath)
		if err == nil {
			return control, nil
		}
		time.Sleep(20 * time.Millisecond)
	}
}

// NewControl opens another connection to the control socket, for a scenario
// that shows what ends with a connection. The connection ends with the test.
func (b *Backend) NewControl() *Control {
	b.t.Helper()
	control, err := dialControl(b.t, filepath.Join(b.h.Root, "control.sock"))
	if err != nil {
		b.t.Fatal(err)
	}
	b.t.Cleanup(control.Close)
	return control
}

// Stop asks the backend to stop, as an operator does, and waits for it to shut
// down; a shutdown that fails fails the test.
func (b *Backend) Stop() {
	b.t.Helper()
	code := b.Shutdown()
	if code != 0 {
		b.t.Errorf("the backend exited with %d:\n%s", code, b.output.String())
	}
}

// Shutdown asks the backend to stop and waits for it, and answers its exit
// code: 0 for a shutdown that succeeded.
func (b *Backend) Shutdown() int {
	b.t.Helper()
	code := b.Terminate()()
	if code < 0 {
		b.t.Fatalf("the backend did not stop within %s:\n%s", Patience, b.output.String())
	}
	return code
}

// Terminate asks the backend to stop without waiting for it. The function it
// answers waits for the shutdown and answers the backend's exit code, or -1
// once it killed a backend that did not stop within Patience; it fails no test,
// so a goroutine of the scenario may call it.
func (b *Backend) Terminate() func() int {
	if b.ended {
		return func() int { return 0 }
	}
	// A process that ended already has nothing to signal.
	_ = b.cmd.Process.Signal(syscall.SIGTERM)
	return func() int {
		select {
		case <-b.exited:
		case <-time.After(Patience):
			b.kill()
			return -1
		}
		b.finish()
		return b.cmd.ProcessState.ExitCode()
	}
}

// kill ends the backend at once, and is a no-op for one that ended.
func (b *Backend) kill() {
	if b.ended {
		return
	}
	procgroup.Kill(b.cmd)
	<-b.exited
	b.finish()
}

// Kill ends the backend at once, as a crash would.
func (b *Backend) Kill() {
	b.t.Helper()
	b.kill()
}

func (b *Backend) finish() {
	b.ended = true
	if b.Control != nil {
		b.Control.Close()
	}
	b.client.CloseIdleConnections()
}

// An outputBuffer collects what a process prints.
type outputBuffer struct {
	mu   sync.Mutex
	text strings.Builder
}

func (o *outputBuffer) copyFrom(source *os.File) {
	buffer := make([]byte, 4096)
	for {
		n, err := source.Read(buffer)
		if n > 0 {
			o.mu.Lock()
			o.text.Write(buffer[:n])
			o.mu.Unlock()
		}
		if err != nil {
			return
		}
	}
}

func (o *outputBuffer) String() string {
	o.mu.Lock()
	defer o.mu.Unlock()
	return o.text.String()
}

// newClient is an HTTP client that goes through no proxy, follows no redirect
// and keeps no cookie: a scenario sends the session's cookie itself.
func newClient() *http.Client {
	return &http.Client{
		Transport: &http.Transport{Proxy: nil},
		Timeout:   Patience * 3,
		CheckRedirect: func(*http.Request, []*http.Request) error {
			return http.ErrUseLastResponse
		},
	}
}

// Eventually waits until check holds, asking every 20 ms for at most Patience.
func Eventually(t testing.TB, what string, check func() bool) {
	t.Helper()
	deadline := time.Now().Add(Patience)
	for !check() {
		if time.Now().After(deadline) {
			t.Fatalf("never came true: %s", what)
		}
		time.Sleep(20 * time.Millisecond)
	}
}
