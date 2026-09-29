//go:build linux

package machines

import (
	"context"
	"errors"
	"sync"

	"github.com/wspl/demi/go/machines/internal/config"
	"github.com/wspl/demi/go/machines/internal/network"
	"github.com/wspl/demi/go/machines/internal/sandbox"
	"github.com/wspl/demi/go/machines/internal/storage"
	"github.com/wspl/demi/go/machines/internal/tools"
	"github.com/wspl/demi/go/machinesproto"
)

// deviceQueue is how many commands wait for one device's worker. A full queue
// holds back the request that sends; its admission covers the wait.
const deviceQueue = 64

// A Core is what every operation uses and nothing changes after startup.
type Core struct {
	sandbox.Host
	Store *storage.Store
}

// NewCore returns the core for a configuration and its resolved programs.
func NewCore(cfg *config.Config, t *tools.Tools) *Core {
	return &Core{
		Host: sandbox.Host{
			Config:  cfg,
			Tools:   t,
			Runsc:   sandbox.NewRunsc(t, cfg.Runtime(), cfg.Limits != nil),
			Network: network.New(cfg.Subnet, cfg.DNS, cfg.BackendURL, t),
			Cgroups: sandbox.NewCgroups(sandbox.DefaultCgroupRoot),
			Slots:   network.NewSlotPool(cfg.Subnet, cfg.Slots),
		},
		Store: storage.NewStore(cfg.Images()),
	}
}

// A Manager is the manager's devices' workers and the admission gate
// (docs/architecture/concurrency.md § Machine manager): one worker per device
// runs that device's operations in order, and the manager-wide admission gate lets
// reconcile and shutdown wait for the operations in flight.
type Manager struct {
	core      *Core
	base      machinesproto.BaseVersion
	admission *admission
	deaths    chan<- machinesproto.DeviceID
	// ctx is the context of everything the workers run; it ends the wait for
	// every sandbox when the manager is done.
	ctx context.Context

	mu      sync.Mutex
	devices map[machinesproto.DeviceID]chan<- command
	workers sync.WaitGroup
}

// NewManager returns a manager whose new devices start on base and whose workers
// report sandbox deaths to deaths, which the manager closes once its workers have
// ended ([Manager.Close]). ctx must outlive every sandbox: their waits run under
// it.
func NewManager(ctx context.Context, core *Core, base machinesproto.BaseVersion, deaths chan<- machinesproto.DeviceID) *Manager {
	return &Manager{
		core:      core,
		base:      base,
		admission: newAdmission(),
		deaths:    deaths,
		ctx:       ctx,
		devices:   make(map[machinesproto.DeviceID]chan<- command),
	}
}

// worker returns the inbox of device's worker, started on first use.
func (m *Manager) worker(device machinesproto.DeviceID) chan<- command {
	m.mu.Lock()
	defer m.mu.Unlock()
	if inbox, ok := m.devices[device]; ok {
		return inbox
	}
	inbox := make(chan command, deviceQueue)
	worker := newDeviceWorker(m.ctx, device, m.core, m.base, m.deaths)
	m.workers.Add(1)
	go func() {
		defer m.workers.Done()
		worker.run(inbox)
	}()
	m.devices[device] = inbox
	return inbox
}

// send hands a command to device's worker and waits for its answer.
func (m *Manager) send(ctx context.Context, device machinesproto.DeviceID, c command) error {
	answer := make(chan error, 1)
	c.reply = answer
	select {
	case m.worker(device) <- c:
	case <-ctx.Done():
		return ctx.Err()
	}
	select {
	case err := <-answer:
		return err
	case <-ctx.Done():
		return ctx.Err()
	}
}

// run runs a command on device's worker, admitted beside other device
// operations. The admission also covers the wait in the queue.
func (m *Manager) run(ctx context.Context, device machinesproto.DeviceID, c command) error {
	admitted, err := m.admission.enter(ctx)
	if err != nil {
		return err
	}
	defer admitted.Release()
	return m.send(ctx, device, c)
}

// runtimeState reads whether the manager runs a sandbox for device, after the
// device's earlier operations.
func (m *Manager) runtimeState(ctx context.Context, device machinesproto.DeviceID) (machinesproto.RuntimeState, error) {
	admitted, err := m.admission.enter(ctx)
	if err != nil {
		return "", err
	}
	defer admitted.Release()
	answer := make(chan machinesproto.RuntimeState, 1)
	select {
	case m.worker(device) <- command{kind: readRuntimeState, state: answer}:
	case <-ctx.Done():
		return "", ctx.Err()
	}
	select {
	case state := <-answer:
		return state, nil
	case <-ctx.Done():
		return "", ctx.Err()
	}
}

// Reconcile stops and saves every device, recovers what an interrupted operation
// left, and installs the network policy again.
func (m *Manager) Reconcile(ctx context.Context) error {
	whole, err := m.admission.exclusive(ctx)
	if err != nil {
		return err
	}
	defer whole.Release()
	if err := m.drain(ctx); err != nil {
		return err
	}
	if err := fenceAndSave(ctx, m.core); err != nil {
		return err
	}
	return m.core.Network.Prepare(ctx)
}

// Close is shutdown: it waits for the operations in flight, stops and saves every
// device, then refuses every request still waiting, so none can start a sandbox
// after the drain. Then the workers end and the death channel closes.
func (m *Manager) Close(ctx context.Context) error {
	whole, err := m.admission.exclusive(ctx)
	if err != nil {
		return err
	}
	defer whole.Release()
	drained := m.drain(ctx)
	m.admission.close()
	m.stopWorkers()
	return drained
}

// stopWorkers ends every worker and closes the death channel. The caller holds
// the whole gate and the gate is closed, so no request sends to a worker.
func (m *Manager) stopWorkers() {
	m.mu.Lock()
	for _, inbox := range m.devices {
		close(inbox)
	}
	m.mu.Unlock()
	m.workers.Wait()
	close(m.deaths)
}

// drain hibernates every known device at once; the caller holds the whole gate,
// so no worker is running an operation.
func (m *Manager) drain(ctx context.Context) error {
	m.mu.Lock()
	devices := make([]machinesproto.DeviceID, 0, len(m.devices))
	for device := range m.devices {
		devices = append(devices, device)
	}
	m.mu.Unlock()
	outcomes := make([]error, len(devices))
	var wg sync.WaitGroup
	for i, device := range devices {
		wg.Add(1)
		go func() {
			defer wg.Done()
			outcomes[i] = m.send(ctx, device, command{kind: hibernate})
		}()
	}
	wg.Wait()
	var failures []error
	for _, outcome := range outcomes {
		if outcome != nil {
			failures = append(failures, outcome)
		}
	}
	if len(failures) > 0 {
		return shutdownFailed(failures)
	}
	return nil
}

// Handle runs one call and returns its result as the ok reply carries it: nil for
// null, a base version, a generation record or nil, a runtime state.
func (m *Manager) Handle(ctx context.Context, call machinesproto.Call) (any, error) {
	switch call := call.(type) {
	case machinesproto.ReconcileParams:
		return nil, m.Reconcile(ctx)
	case machinesproto.CurrentBaseVersionParams:
		return m.base, nil
	case machinesproto.ImageStateParams:
		device, err := machinesproto.ParseDeviceID(call.DeviceID)
		if err != nil {
			return nil, err
		}
		state, err := m.core.Store.Read(device)
		if err != nil {
			return nil, err
		}
		return state, nil
	case machinesproto.RuntimeStateParams:
		device, err := machinesproto.ParseDeviceID(call.DeviceID)
		if err != nil {
			return nil, err
		}
		return m.runtimeState(ctx, device)
	case machinesproto.WakeParams:
		device, err := machinesproto.ParseDeviceID(call.DeviceID)
		if err != nil {
			return nil, err
		}
		return nil, m.run(ctx, device, command{kind: wake, boot: call.Boot})
	case machinesproto.HibernateParams:
		device, err := machinesproto.ParseDeviceID(call.DeviceID)
		if err != nil {
			return nil, err
		}
		return nil, m.run(ctx, device, command{kind: hibernate})
	case machinesproto.CheckpointParams:
		device, err := machinesproto.ParseDeviceID(call.DeviceID)
		if err != nil {
			return nil, err
		}
		return nil, m.run(ctx, device, command{kind: checkpoint})
	case machinesproto.GrowVolumeParams:
		device, err := machinesproto.ParseDeviceID(call.DeviceID)
		if err != nil {
			return nil, err
		}
		return nil, m.run(ctx, device, command{kind: grow, volume: call.Volume, bytes: call.Bytes})
	case machinesproto.ResetParams:
		device, err := machinesproto.ParseDeviceID(call.DeviceID)
		if err != nil {
			return nil, err
		}
		base, err := machinesproto.ParseBaseVersion(call.BaseVersion)
		if err != nil {
			return nil, err
		}
		return nil, m.run(ctx, device, command{kind: reset, operation: call.OperationID, base: base})
	}
	return nil, errors.New("not an operation of this wire")
}
