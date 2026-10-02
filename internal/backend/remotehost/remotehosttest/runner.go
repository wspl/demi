package remotehosttest

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
	"golang.org/x/net/websocket"
)

// FixtureOptions supplies a runner's environment, callback commands and optional wire tap.
type FixtureOptions struct {
	Env      map[string]string
	Commands *host.CommandSet
	Tap      chan<- runnerwire.Outbound
}

// RunnerFixture owns a real runner connected to a backend end of its own.
// Construction registers cleanup for the process, server, connections and pipes.
type RunnerFixture struct {
	process     *RunnerProcess
	pipes       *remotehost.Pipes
	policy      *CommandPolicy
	server      *httptest.Server
	ctx         context.Context
	cancel      context.CancelFunc
	mu          sync.Mutex // Protects device snapshots and change publication.
	device      remotehost.DeviceLink
	closed      bool
	changed     chan struct{}
	connections sync.WaitGroup
	admission   gates.Serial
	tap         chan<- runnerwire.Outbound
}

// StartRunnerFixture starts the backend and runner and waits until it is online.
func StartRunnerFixture(ctx context.Context, t testing.TB, options FixtureOptions) (*RunnerFixture, error) {
	lifetime, cancel := context.WithCancel(context.Background())
	f := &RunnerFixture{pipes: remotehost.NewPipes(remotehost.Arrival), policy: NewCommandPolicy(options.Commands), ctx: lifetime, cancel: cancel, changed: make(chan struct{}), tap: options.Tap}
	t.Cleanup(func() {
		if err := f.Stop(context.Background()); err != nil {
			t.Error(err)
		}
	})
	mux := http.NewServeMux()
	mux.Handle("GET /api/runner", websocket.Server{Handler: f.adopt})
	mux.HandleFunc("PUT /api/pipes/{id}", f.pipeSource)
	mux.HandleFunc("GET /api/pipes/{id}", f.pipeSink)
	f.server = httptest.NewServer(mux)
	processOptions := DefaultRunnerProcessOptions()
	processOptions.Token = new(fixtureToken)
	processOptions.Env = host.SpawnEnv{Mode: host.Overlay, Values: make(map[string]*string)}
	for key, value := range options.Env {
		processOptions.Env.Values[key] = new(value)
	}
	process, err := StartRunnerProcess(ctx, t, f.server.URL, processOptions)
	if err != nil {
		return nil, err
	}
	f.process = process
	onlineCtx, stop := context.WithTimeout(ctx, 15*time.Second)
	defer stop()
	if _, err := f.Link(onlineCtx); err != nil {
		return nil, fmt.Errorf("the runner did not come online:\n%s: %w", f.Log(), err)
	}
	return f, nil
}

// Home returns the runner's private home path.
func (f *RunnerFixture) Home() string {
	return f.process.Home()
}

// JobRoot returns the runner's job directory root.
func (f *RunnerFixture) JobRoot() string {
	return filepath.Join(f.process.StateDir(), "jobs")
}

// JobDirectories lists the runner's retained job directories.
func (f *RunnerFixture) JobDirectories(ctx context.Context) ([]string, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(f.JobRoot())
	if err != nil {
		return nil, nil
	}
	var paths []string
	for _, entry := range entries {
		if entry.IsDir() {
			paths = append(paths, filepath.Join(f.JobRoot(), entry.Name()))
		}
	}
	return paths, nil
}

// Host returns the device Host rooted at the runner's home.
func (f *RunnerFixture) Host() *remotehost.Host {
	return f.HostAt(f.Home())
}

// HostAt returns the device Host with the supplied working directory.
func (f *RunnerFixture) HostAt(cwd string) *remotehost.Host {
	return remotehost.NewHost(host.Key(TestDeviceID+":"+cwd), cwd, func() remotehost.DeviceLink {
		f.mu.Lock()
		defer f.mu.Unlock()
		return f.device
	}, nil)
}

// Pipes returns the fixture's pipe broker.
func (f *RunnerFixture) Pipes() *remotehost.Pipes {
	return f.pipes
}

// Policy returns the fixture's callback policy and storage.
func (f *RunnerFixture) Policy() *CommandPolicy {
	return f.policy
}

// Link waits for the runner's live connection.
func (f *RunnerFixture) Link(ctx context.Context) (*remotehost.Link, error) {
	for {
		f.mu.Lock()
		link := f.device.Link
		changed := f.changed
		f.mu.Unlock()
		if link != nil && !link.IsClosed() {
			return link, nil
		}
		select {
		case <-changed:
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-f.ctx.Done():
			return nil, io.EOF
		}
	}
}

// Offline waits for the runner to disconnect.
func (f *RunnerFixture) Offline(ctx context.Context) error {
	for {
		f.mu.Lock()
		link := f.device.Link
		changed := f.changed
		f.mu.Unlock()
		if link == nil {
			return nil
		}
		select {
		case <-changed:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
}

// Command prepares a CLI command using the fixture's runner state.
// The caller owns starting and waiting for it.
func (f *RunnerFixture) Command(ctx context.Context) *exec.Cmd {
	return f.process.Command(ctx)
}

// Log returns the runner's captured output.
func (f *RunnerFixture) Log() string {
	return f.process.Output()
}

// Stop stops the runner and backend and joins all owned work.
func (f *RunnerFixture) Stop(ctx context.Context) error {
	var result error
	if f.process != nil {
		result = f.process.Stop(context.WithoutCancel(ctx))
	}
	f.mu.Lock()
	f.closed = true
	f.mu.Unlock()
	f.cancel()
	if f.server != nil {
		f.server.Close()
	}
	result = errors.Join(result, f.pipes.Close(context.WithoutCancel(ctx)))
	f.connections.Wait()
	return result
}
