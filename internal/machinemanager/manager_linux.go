//go:build linux

package machinemanager

import (
	"context"
	"encoding/json"
	"errors"
	"sync"

	"github.com/wspl/demi/internal/machinemanagerproto"
)

// Manager owns admission and one worker per device. Close drains and joins them.
type Manager struct {
	core       *Core
	base       machinemanagerproto.BaseVersion
	admission  Admission
	mu         sync.Mutex
	devices    map[machinemanagerproto.DeviceID]*deviceWorker
	workers    sync.WaitGroup
	deaths     chan<- machinemanagerproto.DeviceID
	stopping   chan struct{}
	stopDeaths sync.Once
}

// NewManager starts with no device workers; workers are created on first use.
func NewManager(core *Core, base machinemanagerproto.BaseVersion, deaths chan<- machinemanagerproto.DeviceID) *Manager {
	return &Manager{
		core:     core,
		base:     base,
		devices:  make(map[machinemanagerproto.DeviceID]*deviceWorker),
		deaths:   deaths,
		stopping: make(chan struct{}),
	}
}

func (m *Manager) worker(id machinemanagerproto.DeviceID) *deviceWorker {
	m.mu.Lock()
	defer m.mu.Unlock()
	if w := m.devices[id]; w != nil {
		return w
	}
	w := newDeviceWorker(m, id)
	m.devices[id] = w
	m.workers.Go(func() {
		w.run(context.Background())
	})
	return w
}

// Reconcile exclusively drains devices, recovers working pairs and installs policy.
func (m *Manager) Reconcile(ctx context.Context) error {
	release, err := m.admission.Exclusive(ctx)
	if err != nil {
		return err
	}
	defer release()
	ctx = context.WithoutCancel(ctx)
	if err = m.drain(ctx); err != nil {
		return err
	}
	if err = FenceAndSave(ctx, m.core); err != nil {
		return err
	}
	return m.core.Network.Prepare(ctx)
}

// Close waits for admitted work, drains devices and refuses later requests.
func (m *Manager) Close(ctx context.Context) error {
	// The server has stopped consuming deaths. Unblock workers before waiting
	// for admission, including a checkpoint currently reporting a failed thaw.
	m.stopDeaths.Do(func() {
		close(m.stopping)
	})
	release, err := m.admission.Exclusive(ctx)
	if err != nil {
		return err
	}
	defer release()
	err = m.drain(context.WithoutCancel(ctx))
	m.admission.Close()
	m.mu.Lock()
	for _, w := range m.devices {
		close(w.queue)
	}
	m.mu.Unlock()
	m.workers.Wait()
	return err
}

func (m *Manager) drain(ctx context.Context) error {
	m.mu.Lock()
	workers := make([]*deviceWorker, 0, len(m.devices))
	for _, w := range m.devices {
		workers = append(workers, w)
	}
	m.mu.Unlock()
	results := make([]error, len(workers))
	var jobs sync.WaitGroup
	for i, w := range workers {
		jobs.Go(func() {
			_, results[i] = w.call(ctx, &machinemanagerproto.Hibernate{})
		})
	}
	jobs.Wait()
	if err := errors.Join(results...); err != nil {
		return &OperationError{Message: "Cloud shutdown failed; working state retained", Cause: err}
	}
	return nil
}

// Handle runs a decoded machine request. Once admitted, its work completes despite cancellation.
func (m *Manager) Handle(ctx context.Context, call machinemanagerproto.Call) (json.RawMessage, error) {
	var id string
	switch op := call.(type) {
	case *machinemanagerproto.Reconcile:
		return json.RawMessage("null"), m.Reconcile(ctx)
	case *machinemanagerproto.CurrentBaseVersion:
		return m.base.MarshalJSON()
	case *machinemanagerproto.ImageState:
		device, err := machinemanagerproto.ParseDeviceID(op.Params.DeviceID)
		if err != nil {
			return nil, err
		}
		state, found, err := m.core.Store.Read(ctx, device)
		if err != nil {
			return nil, err
		}
		if !found {
			return json.RawMessage("null"), nil
		}
		return state.MarshalJSON()
	case *machinemanagerproto.RuntimeStateCall:
		id = op.Params.DeviceID
	case *machinemanagerproto.Wake:
		id = op.Params.DeviceID
	case *machinemanagerproto.Hibernate:
		id = op.Params.DeviceID
	case *machinemanagerproto.Checkpoint:
		id = op.Params.DeviceID
	case *machinemanagerproto.GrowVolume:
		id = op.Params.DeviceID
	case *machinemanagerproto.Reset:
		id = op.Params.DeviceID
		if _, err := machinemanagerproto.ParseBaseVersion(op.Params.BaseVersion); err != nil {
			return nil, err
		}
	}
	device, err := machinemanagerproto.ParseDeviceID(id)
	if err != nil {
		return nil, err
	}
	release, err := m.admission.Enter(ctx)
	if err != nil {
		return nil, err
	}
	defer release()
	return m.worker(device).call(context.WithoutCancel(ctx), call)
}

// OperationError carries an operation's summary and all underlying failures.
type OperationError struct {
	// Message is the operation summary carried on the wire.
	Message string
	// Cause preserves the underlying failures.
	Cause error
}

// Error returns the failure message.
func (e *OperationError) Error() string {
	return e.Message
}

// Unwrap returns the underlying failure.
func (e *OperationError) Unwrap() error {
	return e.Cause
}
