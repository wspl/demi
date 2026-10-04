package backendtest

import (
	"context"
	"errors"
	"maps"
	"net"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/backend/usershard/usershardtest"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/machinemanagerproto"
	"github.com/wspl/demi/internal/webapiproto"
)

// Base is the base every scripted device boots from.
const Base = "test-base"

// MachineScript is what the manager does differently, as a test sets it.
// Configure between calls; running requests retain the script they admitted.
type MachineScript struct {
	// FailReset makes the next reset fail with this message.
	FailReset *string
	// SilentWake makes wakes start no runner, as a guest that never connects.
	SilentWake bool
	// FailHibernate makes the next hibernate fail with this message.
	FailHibernate *string
	// CloudEnv replaces the environment of newly started Cloud runners;
	// nil inherits the test process's environment.
	CloudEnv map[string]string
	// Artifacts names the artifact cache each Cloud runner that starts from now on uses,
	// as `DEMI_ARTIFACTS`; none keeps its own in its state.
	Artifacts *string
}

// ScriptedManager owns a local manager socket, request workers and real runners.
// Test cleanup calls Close, which cancels socket work and joins every worker.
type ScriptedManager struct {
	socket      string
	listener    net.Listener
	ctx         context.Context
	cancel      context.CancelFunc
	done        chan struct{}
	workers     sync.WaitGroup
	mu          sync.Mutex
	calls       []machineArrival
	changed     chan struct{}
	guests      map[string]*machineGuest
	script      MachineScript
	connections map[*machineConnection]struct{}
	turns       gates.KeyedSerial[string]
	resets      usershardtest.StepHolds[string]
	startRunner func(context.Context, string, remotehosttest.RunnerProcessOptions) (*remotehosttest.RunnerProcess, error)
	err         error
}
type machineArrival struct {
	name string
	at   time.Time
}
type machineGuest struct {
	runner     *remotehosttest.RunnerProcess
	image      machinemanagerproto.MachineImageState
	generation uint64
}

// StartScriptedManager starts a manager with temporary socket and device data.
func StartScriptedManager(ctx context.Context, t testing.TB) (*ScriptedManager, error) {
	t.Helper()
	manager, err := startScriptedManager(ctx)
	if err != nil {
		return nil, err
	}
	t.Cleanup(func() {
		if err := manager.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return manager, nil
}

// startScriptedManager composes the manager protocol with the existing runner fixture.
func startScriptedManager(ctx context.Context) (*ScriptedManager, error) {
	root := ""
	if runtime.GOOS != "windows" {
		root = "/tmp"
	}
	directory, err := os.MkdirTemp(root, "machines-")
	if err != nil {
		return nil, err
	}
	start := func(
		ctx context.Context,
		backend string,
		options remotehosttest.RunnerProcessOptions,
	) (*remotehosttest.RunnerProcess, error) {
		runnerDir, err := os.MkdirTemp(directory, "runner-")
		if err != nil {
			return nil, err
		}
		return remotehosttest.StartOwnedRunnerProcess(ctx, runnerDir, backend, options)
	}
	socket := filepath.Join(directory, "machines.sock")
	listener, err := (&net.ListenConfig{}).Listen(ctx, "unix", socket)
	if err != nil {
		return nil, errors.Join(err, os.RemoveAll(directory))
	}
	life, cancel := context.WithCancel(ctx)
	m := &ScriptedManager{
		socket:      socket,
		listener:    listener,
		ctx:         life,
		cancel:      cancel,
		done:        make(chan struct{}),
		changed:     make(chan struct{}),
		guests:      make(map[string]*machineGuest),
		connections: make(map[*machineConnection]struct{}),
		startRunner: start,
	}
	m.workers.Go(m.serve)
	go func() {
		<-life.Done()
		// Closing a listener/socket is teardown; their read loops own diagnostics.
		_ = listener.Close()
		m.mu.Lock()
		sockets := make([]net.Conn, 0, len(m.connections))
		for connection := range m.connections {
			sockets = append(sockets, connection.socket)
		}
		m.mu.Unlock()
		for _, socket := range sockets {
			_ = socket.Close()
		}
		m.workers.Wait()
		for _, device := range m.Devices() {
			m.err = errors.Join(m.err, m.StopQuietly(context.Background(), device))
		}
		m.err = errors.Join(m.err, os.RemoveAll(directory))
		close(m.done)
	}()
	return m, nil
}

// Socket returns the machine-manager Unix socket path.
func (m *ScriptedManager) Socket() string {
	return m.socket
}

// Calls snapshots manager request names in arrival order.
func (m *ScriptedManager) Calls() []string {
	m.mu.Lock()
	defer m.mu.Unlock()
	calls := make([]string, len(m.calls))
	for i, arrival := range m.calls {
		calls[i] = arrival.name
	}
	return calls
}

// Arrival waits until call arrives and returns its recorded arrival time.
func (m *ScriptedManager) Arrival(ctx context.Context, call string) (time.Time, error) {
	for {
		m.mu.Lock()
		for _, arrival := range m.calls {
			if arrival.name == call {
				m.mu.Unlock()
				return arrival.at, nil
			}
		}
		changed := m.changed
		m.mu.Unlock()
		select {
		case <-ctx.Done():
			return time.Time{}, ctx.Err()
		case <-m.done:
			return time.Time{}, net.ErrClosed
		case <-changed:
		}
	}
}

// Count returns how many times call arrived.
func (m *ScriptedManager) Count(call string) int {
	count := 0
	for _, name := range m.Calls() {
		if name == call {
			count++
		}
	}
	return count
}

// SetScript replaces future requests' settings, copying mutable values.
func (m *ScriptedManager) SetScript(script MachineScript) {
	script.CloudEnv = maps.Clone(script.CloudEnv)
	if script.FailReset != nil {
		value := *script.FailReset
		script.FailReset = &value
	}
	if script.FailHibernate != nil {
		value := *script.FailHibernate
		script.FailHibernate = &value
	}
	if script.Artifacts != nil {
		value := *script.Artifacts
		script.Artifacts = &value
	}
	m.mu.Lock()
	m.script = script
	m.mu.Unlock()
}

// Devices returns the manager's known device IDs in lexical order.
func (m *ScriptedManager) Devices() []webapiproto.DeviceID {
	m.mu.Lock()
	defer m.mu.Unlock()
	devices := make([]webapiproto.DeviceID, 0, len(m.guests))
	for device := range m.guests {
		devices = append(devices, webapiproto.DeviceID(device))
	}
	slices.Sort(devices)
	return devices
}

// Running reports whether the scripted device has a running guest.
func (m *ScriptedManager) Running(device webapiproto.DeviceID) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	guest := m.guests[string(device)]
	return guest != nil && guest.runner != nil && guest.runner.Running()
}

// State returns the device runner's installation-state directory.
func (m *ScriptedManager) State(device webapiproto.DeviceID) string {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.guests[string(device)].runner.StateDir()
}

// Home returns the device's preserved home directory.
func (m *ScriptedManager) Home(device webapiproto.DeviceID) string {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.guests[string(device)].runner.Home()
}

// Artifacts returns the device runner's artifact-cache directory.
func (m *ScriptedManager) Artifacts(device webapiproto.DeviceID) string {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.script.Artifacts != nil {
		return *m.script.Artifacts
	}
	return filepath.Join(m.guests[string(device)].runner.StateDir(), "artifacts")
}

// Kill stops a device runner and reports its death to every live connection.
func (m *ScriptedManager) Kill(ctx context.Context, device webapiproto.DeviceID) error {
	permit, err := m.turns.Acquire(ctx, string(device))
	if err != nil {
		return err
	}
	defer permit.Release()
	runner := m.takeRunner(string(device))
	if runner == nil {
		return nil
	}
	err = runner.Kill(ctx)
	m.putRunner(string(device), runner)
	m.mu.Lock()
	for connection := range m.connections {
		// A slow subscriber loses its oldest death when a new one arrives.
		select {
		case connection.deaths <- string(device):
		default:
			select {
			case <-connection.deaths:
			default:
			}
			connection.deaths <- string(device)
		}
	}
	m.mu.Unlock()
	return err
}

// StopQuietly stops a device runner without reporting a death event.
func (m *ScriptedManager) StopQuietly(ctx context.Context, device webapiproto.DeviceID) error {
	permit, err := m.turns.Acquire(ctx, string(device))
	if err != nil {
		return err
	}
	defer permit.Release()
	return m.stopRunner(ctx, string(device))
}

// Close stops and joins manager connections, handlers and runner processes.
func (m *ScriptedManager) Close(ctx context.Context) error {
	m.cancel()
	select {
	case <-m.done:
		return m.err
	case <-ctx.Done():
		return ctx.Err()
	}
}

// HoldReset holds resets at their disk step until Release or test cleanup.
func (m *ScriptedManager) HoldReset(t testing.TB) *StepHold {
	return m.resets.Hold(t, "reset")
}

// record publishes a manager operation arrival before its scripted work starts.
func (m *ScriptedManager) record(name string) {
	m.mu.Lock()
	m.calls = append(m.calls, machineArrival{name: name, at: time.Now()})
	close(m.changed)
	m.changed = make(chan struct{})
	m.mu.Unlock()
}

// takeRunner transfers exclusive process access out of shared manager state.
func (m *ScriptedManager) takeRunner(device string) *remotehosttest.RunnerProcess {
	m.mu.Lock()
	defer m.mu.Unlock()
	guest := m.guests[device]
	if guest == nil {
		return nil
	}
	runner := guest.runner
	guest.runner = nil
	return runner
}

// putRunner returns a process after its owner's operation has finished.
func (m *ScriptedManager) putRunner(device string, runner *remotehosttest.RunnerProcess) {
	m.mu.Lock()
	m.guests[device].runner = runner
	m.mu.Unlock()
}

// stopRunner models the loss of transient guest state while preserving home.
func (m *ScriptedManager) stopRunner(ctx context.Context, device string) error {
	runner := m.takeRunner(device)
	if runner == nil {
		return nil
	}
	defer m.putRunner(device, runner)
	if err := runner.Stop(ctx); err != nil {
		return err
	}
	return runner.ClearRunState(ctx)
}
