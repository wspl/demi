package scripted

import (
	"bufio"
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/go/backendtest/runnerproc"
	"github.com/wspl/demi/go/machinesproto"
)

// Base is the base every device boots from.
const Base = "test-base"

// volumeBytes is the capacity of each of a device's filesystems until it grows.
const volumeBytes = 1 << 30

// A Script is what the manager does differently, as a test sets it.
type Script struct {
	// FailReset, when set, fails the next reset with the message.
	FailReset *string
	// HoldReset holds each reset at its disk step: Arrived is told once one
	// arrives there, and the reset goes on once Proceed is.
	HoldReset *ResetHold
	// SilentWake makes wakes start no runner, as a guest that never connects.
	SilentWake bool
	// FailHibernate, when set, fails the next hibernate with the message.
	FailHibernate *string
	// CloudEnv is the whole environment of each Cloud runner that starts from
	// now on, as a guest image gives it; nil inherits the test process's.
	CloudEnv []string
}

// A ResetHold pauses resets at their disk step.
type ResetHold struct {
	arrived chan struct{}
	proceed chan struct{}
}

// NewResetHold returns a hold that pauses resets.
func NewResetHold() *ResetHold {
	return &ResetHold{arrived: make(chan struct{}, 64), proceed: make(chan struct{}, 64)}
}

// Arrived receives once for each reset that reached the hold.
func (h *ResetHold) Arrived() <-chan struct{} { return h.arrived }

// Proceed lets one waiting reset, or the next to arrive, go on.
func (h *ResetHold) Proceed() { h.proceed <- struct{}{} }

// A guest is a device's storage and its sandbox.
type guest struct {
	runner      *runnerproc.Process
	image       machinesproto.ImageState
	generations uint64
}

// A Manager is a machine manager the tests script: it serves the manager's
// socket with machines-protocol, as the real manager does, and runs each
// Cloud's sandbox as a real runner process whose home survives a stop, a wake
// and a reset. Operations of one device run one at a time, in arrival order; a
// device's first wake makes its storage. It records every call, and a test can
// hold a reset, fail one, keep a wake from starting its runner, or kill a
// runner as a crash would, which the manager reports as a death to every
// connection. It mounts no image and isolates nothing: a stop clears the
// runner's state but its log and its job directories, as a Cloud's /run/demi
// goes with it while its system image, which holds those two, stays; a reset
// clears them too; both keep its home.
type Manager struct {
	t       testing.TB
	runner  string
	root    string
	socket  string
	closing context.CancelFunc

	mu          sync.Mutex
	calls       []call
	guests      map[string]*guest
	workers     map[string]chan struct{}
	script      Script
	connections map[*connection]struct{}
	arrived     chan struct{}
}

type call struct {
	name string
	at   time.Time
}

// StartManager starts a manager that runs its Clouds' runners with the program
// runner. It stops with the test.
func StartManager(t testing.TB, runner string) *Manager {
	t.Helper()
	// A socket path is short: the limit is about a hundred bytes.
	root, err := os.MkdirTemp("", "demi-manager-")
	if err != nil {
		t.Fatal(err)
	}
	socket := filepath.Join(root, "machines.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		t.Fatal(err)
	}
	ctx, closing := context.WithCancel(context.Background())
	m := &Manager{
		t:           t,
		runner:      runner,
		root:        root,
		socket:      socket,
		closing:     closing,
		guests:      map[string]*guest{},
		workers:     map[string]chan struct{}{},
		connections: map[*connection]struct{}{},
		arrived:     make(chan struct{}),
	}
	served := make(chan struct{})
	go func() {
		defer close(served)
		m.serve(ctx, listener)
	}()
	t.Cleanup(func() {
		closing()
		// A listener that is already closed has nothing to close.
		_ = listener.Close()
		<-served
		m.mu.Lock()
		guests := m.guests
		m.mu.Unlock()
		for _, guest := range guests {
			if guest.runner != nil {
				guest.runner.Remove()
			}
		}
		_ = os.RemoveAll(root)
	})
	return m
}

// Socket is the manager's socket, which the backend is configured with.
func (m *Manager) Socket() string { return m.socket }

// Calls returns every call so far, such as wake:<device>, in arrival order.
func (m *Manager) Calls() []string {
	m.mu.Lock()
	defer m.mu.Unlock()
	names := make([]string, len(m.calls))
	for i, c := range m.calls {
		names[i] = c.name
	}
	return names
}

// Count returns how many calls so far are name.
func (m *Manager) Count(name string) int {
	count := 0
	for _, recorded := range m.Calls() {
		if recorded == name {
			count++
		}
	}
	return count
}

// Arrival returns when the call first arrived, such as the hibernate:<device>
// that stops a Cloud, waiting for it for at most patience; a call that never
// comes fails the test as hung.
func (m *Manager) Arrival(name string, patience time.Duration) time.Time {
	m.t.Helper()
	timeout := time.After(patience)
	for {
		m.mu.Lock()
		next := m.arrived
		for _, c := range m.calls {
			if c.name == name {
				m.mu.Unlock()
				return c.at
			}
		}
		m.mu.Unlock()
		select {
		case <-next:
		case <-timeout:
			m.t.Fatalf("the manager never received %s: %v", name, m.Calls())
		}
	}
}

// Configure changes what the manager does from now on.
func (m *Manager) Configure(change func(*Script)) {
	m.mu.Lock()
	defer m.mu.Unlock()
	change(&m.script)
}

// Devices returns the devices whose storage the manager made.
func (m *Manager) Devices() []string {
	m.mu.Lock()
	defer m.mu.Unlock()
	devices := make([]string, 0, len(m.guests))
	for device := range m.guests {
		devices = append(devices, device)
	}
	sort.Strings(devices)
	return devices
}

// Running reports whether the device's runner runs.
func (m *Manager) Running(device string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	guest := m.guests[device]
	return guest != nil && guest.runner != nil && guest.runner.Running()
}

// State returns the runner's installation state, /run/demi on a real Cloud,
// whose jobs/ stands for the job root on the Cloud's system image.
func (m *Manager) State(device string) string {
	m.t.Helper()
	return m.runnerOf(device).StateDir()
}

// Home returns the home the device's runner reports, where the Cloud's files
// are.
func (m *Manager) Home(device string) string {
	m.t.Helper()
	return m.runnerOf(device).Home()
}

func (m *Manager) runnerOf(device string) *runnerproc.Process {
	m.t.Helper()
	m.mu.Lock()
	defer m.mu.Unlock()
	guest := m.guests[device]
	if guest == nil || guest.runner == nil {
		m.t.Fatalf("the device %s has no runner", device)
	}
	return guest.runner
}

// Kill kills the device's runner as a crash would, and reports its death to
// every connection.
func (m *Manager) Kill(device string) {
	release := m.acquire(device)
	defer release()
	m.mu.Lock()
	found := m.guests[device]
	var runner *runnerproc.Process
	if found != nil {
		runner = found.runner
	}
	m.mu.Unlock()
	if runner == nil {
		return
	}
	runner.Kill()
	m.broadcastDeath(device)
}

// StopQuietly stops the device's runner the way a manager restart does:
// without a death event.
func (m *Manager) StopQuietly(device string) {
	release := m.acquire(device)
	defer release()
	m.stopRunner(device)
}

// worker is the device's turn: one operation runs at a time.
func (m *Manager) worker(device string) chan struct{} {
	m.mu.Lock()
	defer m.mu.Unlock()
	turn, ok := m.workers[device]
	if !ok {
		turn = make(chan struct{}, 1)
		m.workers[device] = turn
	}
	return turn
}

// acquire waits for the device's turn and answers the function that ends it.
func (m *Manager) acquire(device string) func() {
	turn := m.worker(device)
	turn <- struct{}{}
	return func() { <-turn }
}

// record notes a call as it arrives.
func (m *Manager) record(name string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.calls = append(m.calls, call{name: name, at: time.Now()})
	close(m.arrived)
	m.arrived = make(chan struct{})
}

// A connection is one backend connection to the manager.
type connection struct {
	conn net.Conn
	mu   sync.Mutex
}

// send writes one line; a connection that closed discards it.
func (c *connection) send(line []byte) {
	c.mu.Lock()
	defer c.mu.Unlock()
	_, _ = c.conn.Write(line)
}

func (m *Manager) broadcastDeath(device string) {
	line, err := machinesproto.EncodeResponse(machinesproto.Death{DeviceID: device})
	if err != nil {
		m.t.Errorf("the manager cannot encode a death: %v", err)
		return
	}
	m.mu.Lock()
	connections := make([]*connection, 0, len(m.connections))
	for c := range m.connections {
		connections = append(connections, c)
	}
	m.mu.Unlock()
	for _, c := range connections {
		c.send(line)
	}
}

func (m *Manager) serve(ctx context.Context, listener net.Listener) {
	var connections sync.WaitGroup
	defer connections.Wait()
	for {
		conn, err := listener.Accept()
		if err != nil {
			return
		}
		c := &connection{conn: conn}
		m.mu.Lock()
		m.connections[c] = struct{}{}
		m.mu.Unlock()
		connections.Add(1)
		go func() {
			defer connections.Done()
			m.serveConnection(ctx, c)
			m.mu.Lock()
			delete(m.connections, c)
			m.mu.Unlock()
		}()
		go func() {
			// The connection ends with the manager.
			<-ctx.Done()
			_ = conn.Close()
		}()
	}
}

// serveConnection runs a connection's requests concurrently and answers them
// as they finish. A line that is not a request drops the connection, as the
// real manager does.
func (m *Manager) serveConnection(ctx context.Context, c *connection) {
	defer c.conn.Close()
	var requests sync.WaitGroup
	// The replies still owed are written before the connection ends.
	defer requests.Wait()
	reader := bufio.NewReaderSize(c.conn, 1<<16)
	for {
		line, err := reader.ReadBytes('\n')
		if err != nil {
			return
		}
		line = line[:len(line)-1]
		if len(line) == 0 {
			continue
		}
		request, err := machinesproto.DecodeRequest(line)
		if err != nil {
			return
		}
		requests.Add(1)
		go func() {
			defer requests.Done()
			var response machinesproto.Response
			result, err := m.handle(ctx, request.Call)
			if err != nil {
				response = machinesproto.Failure{ID: request.ID, Message: err.Error()}
			} else {
				response = machinesproto.OK{ID: request.ID, Result: result}
			}
			encoded, err := machinesproto.EncodeResponse(response)
			if err != nil {
				m.t.Errorf("the manager cannot encode a reply: %v", err)
				return
			}
			c.send(encoded)
		}()
	}
}

var (
	nullResult = jsontext.Value("null")
)

func (m *Manager) handle(ctx context.Context, request machinesproto.Call) (jsontext.Value, error) {
	switch params := request.(type) {
	case machinesproto.ReconcileParams:
		m.record("reconcile")
		m.mu.Lock()
		devices := make([]string, 0, len(m.guests))
		for device := range m.guests {
			devices = append(devices, device)
		}
		m.mu.Unlock()
		for _, device := range devices {
			func() {
				release := m.acquire(device)
				defer release()
				m.stopRunner(device)
			}()
		}
		return nullResult, nil
	case machinesproto.CurrentBaseVersionParams:
		return json.Marshal(Base)
	case machinesproto.ImageStateParams:
		m.mu.Lock()
		defer m.mu.Unlock()
		guest := m.guests[params.DeviceID]
		if guest == nil {
			return nullResult, nil
		}
		return machinesproto.EncodeImageStateResult(&guest.image)
	case machinesproto.RuntimeStateParams:
		release := m.acquire(params.DeviceID)
		defer release()
		state := machinesproto.Stopped
		if m.Running(params.DeviceID) {
			state = machinesproto.Running
		}
		return json.Marshal(state)
	case machinesproto.WakeParams:
		release := m.acquire(params.DeviceID)
		defer release()
		m.record("wake:" + params.DeviceID)
		return nullResult, m.wake(params)
	case machinesproto.HibernateParams:
		release := m.acquire(params.DeviceID)
		defer release()
		m.record("hibernate:" + params.DeviceID)
		m.mu.Lock()
		failure := m.script.FailHibernate
		m.script.FailHibernate = nil
		m.mu.Unlock()
		if failure != nil {
			return nil, errors.New(*failure)
		}
		m.stopRunner(params.DeviceID)
		return nullResult, nil
	case machinesproto.CheckpointParams:
		release := m.acquire(params.DeviceID)
		defer release()
		m.record("checkpoint:" + params.DeviceID)
		return nullResult, nil
	case machinesproto.GrowVolumeParams:
		release := m.acquire(params.DeviceID)
		defer release()
		m.record(fmt.Sprintf("grow:%s:%s:%d", params.DeviceID, params.Volume, params.Bytes))
		m.mu.Lock()
		defer m.mu.Unlock()
		guest := m.guests[params.DeviceID]
		if guest == nil {
			return nil, errors.New("the device has no storage")
		}
		if params.Bytes > guest.image.Bytes(params.Volume) {
			guest.image = guest.image.WithBytes(params.Volume, params.Bytes)
		}
		return nullResult, nil
	case machinesproto.ResetParams:
		release := m.acquire(params.DeviceID)
		defer release()
		m.record(fmt.Sprintf("reset:%s:%s:%s", params.DeviceID, params.OperationID, params.BaseVersion))
		return nullResult, m.reset(ctx, params)
	}
	return nil, fmt.Errorf("the manager has no operation %T", request)
}

// wake creates first-use storage or recovers it, then starts a sandbox.
func (m *Manager) wake(params machinesproto.WakeParams) error {
	device := params.DeviceID
	m.mu.Lock()
	silent := m.script.SilentWake
	env := slices.Clone(m.script.CloudEnv)
	found := m.guests[device]
	if found == nil {
		found = &guest{image: image(1, nil, volumeBytes, volumeBytes), generations: 1}
		m.guests[device] = found
	}
	runner := found.runner
	m.mu.Unlock()
	backend := string(params.Boot.BackendURL)
	token := params.Boot.DeviceToken.Expose()
	var started *runnerproc.Process
	switch {
	case runner != nil && runner.Running():
		// A sandbox that runs is left as it is.
		started = runner
	case silent:
		// A guest that never connects: its runner does not start.
		started = runner
	case runner != nil:
		if err := runner.StartAgainWithToken(backend, token); err != nil {
			return err
		}
		started = runner
	default:
		process, err := runnerproc.Start(m.runner, backend, m.root, runnerproc.Options{
			Name:    "cloud",
			Env:     env,
			Token:   token,
			Managed: true,
		})
		if err != nil {
			return err
		}
		started = process
	}
	m.mu.Lock()
	found.runner = started
	m.mu.Unlock()
	return nil
}

// reset publishes a clean system with the retained home.
func (m *Manager) reset(ctx context.Context, params machinesproto.ResetParams) error {
	device := params.DeviceID
	m.mu.Lock()
	hold := m.script.HoldReset
	m.mu.Unlock()
	if hold != nil {
		hold.arrived <- struct{}{}
		select {
		case <-hold.proceed:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
	m.mu.Lock()
	failure := m.script.FailReset
	m.script.FailReset = nil
	m.mu.Unlock()
	if failure != nil {
		return errors.New(*failure)
	}
	m.stopRunner(device)
	m.mu.Lock()
	found := m.guests[device]
	var runner *runnerproc.Process
	if found != nil {
		runner = found.runner
	}
	m.mu.Unlock()
	if runner != nil {
		// A reset replaces the system layer, the log and the job directories
		// with it.
		if err := runner.ClearState(); err != nil {
			return err
		}
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	if found != nil {
		found.generations++
		operation := params.OperationID
		found.image = image(found.generations, &operation, found.image.SystemBytes, found.image.HomeBytes)
	}
	return nil
}

// stopRunner stops the device's runner, if it runs, and clears its state but
// its log and its job directories, as a stop takes a Cloud's temporary mounts
// and keeps its system image; the caller holds the device's turn.
func (m *Manager) stopRunner(device string) {
	m.mu.Lock()
	found := m.guests[device]
	var runner *runnerproc.Process
	if found != nil {
		runner = found.runner
	}
	m.mu.Unlock()
	if runner == nil {
		return
	}
	runner.Stop()
	if err := runner.ClearRunState(); err != nil {
		m.t.Errorf("the runner of %s cannot be cleared: %v", device, err)
	}
}

func image(generation uint64, reset *string, system, home uint64) machinesproto.ImageState {
	return machinesproto.ImageState{
		Generation:  machinesproto.GenerationID(fmt.Sprintf("gen-%d", generation)),
		BaseVersion: Base,
		ResetID:     reset,
		SystemBytes: system,
		HomeBytes:   home,
	}
}
