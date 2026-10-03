package backendtest

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"strings"
	"sync"

	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/machinewire"
	"github.com/wspl/demi/internal/webapi"
)

type machineConnection struct {
	socket net.Conn
	deaths chan string
}

// serve accepts manager clients and owns all connection handlers.
func (m *ScriptedManager) serve() {
	for {
		socket, err := m.listener.Accept()
		if err != nil {
			return
		}
		connection := &machineConnection{socket: socket, deaths: make(chan string, 64)}
		m.mu.Lock()
		if m.ctx.Err() != nil {
			m.mu.Unlock()
			_ = socket.Close()
			return
		}
		m.connections[connection] = struct{}{}
		m.mu.Unlock()
		m.workers.Go(func() { m.connection(connection) })
	}
}

// connection validates requests and joins their replies and its one writer.
func (m *ScriptedManager) connection(connection *machineConnection) {
	socket := connection.socket
	defer func() {
		m.mu.Lock()
		delete(m.connections, connection)
		m.mu.Unlock()
		_ = socket.Close() // Closing an already closed fixture connection is harmless.
	}()
	ctx, cancel := context.WithCancel(m.ctx)
	defer cancel()
	outgoing := make(chan machinewire.MachineResponse, 64)
	writerDone := make(chan struct{})
	go func() {
		defer close(writerDone)
		defer cancel()
		defer func() {
			// Unblocks the reader; an already closed socket needs no further cleanup.
			_ = socket.Close()
		}()
		for {
			var message machinewire.MachineResponse
			select {
			case <-ctx.Done():
				return
			case device := <-connection.deaths:
				message = &machinewire.Death{DeviceID: device}
			case reply, ok := <-outgoing:
				if !ok {
					return
				}
				message = reply
			}
			line, err := machinewire.EncodeLine(message)
			if err != nil {
				return
			}
			if _, err := socket.Write(line); err != nil {
				return
			}
		}
	}()
	var requests sync.WaitGroup
	reader := bufio.NewReader(socket)
	for {
		line, err := reader.ReadBytes('\n')
		if err != nil {
			break
		}
		line = []byte(strings.TrimSuffix(string(line), "\n"))
		if len(line) == 0 {
			continue
		}
		request, err := machinewire.DecodeRequest(line)
		if err != nil {
			break
		}
		requests.Go(func() {
			result, err := m.handle(m.ctx, request.Call)
			var response machinewire.MachineResponse = &machinewire.OK{ID: request.ID, Result: result}
			if err != nil {
				response = &machinewire.ErrorResponse{ID: request.ID, Message: err.Error()}
			}
			select {
			case outgoing <- response:
			case <-ctx.Done():
			}
		})
	}
	requests.Wait()
	close(outgoing)
	<-writerDone
}

// handle implements the scripted manager's device operations with per-device admission.
func (m *ScriptedManager) handle(ctx context.Context, call machinewire.MachineCall) (json.RawMessage, error) {
	if _, ok := call.(*machinewire.Reconcile); ok {
		m.record("reconcile")
		for _, device := range m.Devices() {
			if err := m.StopQuietly(ctx, device); err != nil {
				return nil, err
			}
		}
		return json.RawMessage("null"), nil
	}
	var device string
	switch call := call.(type) {
	case *machinewire.Reconcile: // Handled before device admission.
	case *machinewire.CurrentBaseVersion:
		return contract.EncodeJSON(machinewire.BaseVersion(Base))
	case *machinewire.ImageState:
		m.mu.Lock()
		var image *machinewire.MachineImageState
		if guest := m.guests[call.Params.DeviceID]; guest != nil {
			snapshot := guest.image
			image = &snapshot
		}
		m.mu.Unlock()
		return contract.EncodeJSON(image)
	case *machinewire.RuntimeStateCall:
		device = call.Params.DeviceID
	case *machinewire.Wake:
		device = call.Params.DeviceID
	case *machinewire.Hibernate:
		device = call.Params.DeviceID
	case *machinewire.Checkpoint:
		device = call.Params.DeviceID
	case *machinewire.GrowVolume:
		device = call.Params.DeviceID
	case *machinewire.Reset:
		device = call.Params.DeviceID
	}
	permit, err := m.turns.Acquire(ctx, device)
	if err != nil {
		return nil, err
	}
	defer permit.Release()
	switch call := call.(type) {
	case *machinewire.Reconcile, *machinewire.CurrentBaseVersion, *machinewire.ImageState:
		return nil, errors.New("manager operation reached device admission unexpectedly")
	case *machinewire.RuntimeStateCall:
		state := machinewire.RuntimeStateStopped
		if m.Running(webapi.DeviceID(device)) {
			state = machinewire.RuntimeStateRunning
		}
		return contract.EncodeJSON(state)
	case *machinewire.Wake:
		m.record("wake:" + device)
		return json.RawMessage("null"), m.wake(ctx, device, call.Params)
	case *machinewire.Hibernate:
		m.record("hibernate:" + device)
		m.mu.Lock()
		failure := m.script.FailHibernate
		m.script.FailHibernate = nil
		m.mu.Unlock()
		if failure != nil {
			return nil, errors.New(*failure)
		}
		return json.RawMessage("null"), m.stopRunner(ctx, device)
	case *machinewire.Checkpoint:
		m.record("checkpoint:" + device)
	case *machinewire.GrowVolume:
		p := call.Params
		m.record(fmt.Sprintf("grow:%s:%s:%d", device, p.Volume, p.Bytes))
		m.mu.Lock()
		defer m.mu.Unlock()
		guest := m.guests[device]
		if guest == nil {
			return nil, errors.New("the device has no storage")
		}
		if p.Volume == machinewire.VolumeSystem {
			guest.image.SystemBytes = max(guest.image.SystemBytes, p.Bytes)
		} else {
			guest.image.HomeBytes = max(guest.image.HomeBytes, p.Bytes)
		}
	case *machinewire.Reset:
		p := call.Params
		m.record(fmt.Sprintf("reset:%s:%s:%s", device, p.OperationID, p.BaseVersion))
		if err := m.resets.Pass(ctx, "reset"); err != nil {
			return nil, err
		}
		m.mu.Lock()
		failure := m.script.FailReset
		m.script.FailReset = nil
		m.mu.Unlock()
		if failure != nil {
			return nil, errors.New(*failure)
		}
		if err := m.stopRunner(ctx, device); err != nil {
			return nil, err
		}
		runner := m.takeRunner(device)
		if runner != nil {
			err := runner.ClearState(ctx)
			m.putRunner(device, runner)
			if err != nil {
				return nil, err
			}
		}
		m.mu.Lock()
		if guest := m.guests[device]; guest != nil {
			guest.generation++
			guest.image.Generation = machinewire.GenerationID(fmt.Sprintf("gen-%d", guest.generation))
			guest.image.ResetID = &p.OperationID
		}
		m.mu.Unlock()
	}
	return json.RawMessage("null"), nil
}

// wake starts or resumes the device's existing runner without replacing its home.
func (m *ScriptedManager) wake(ctx context.Context, device string, params machinewire.WakeParams) error {
	m.mu.Lock()
	script := m.script
	if m.guests[device] == nil {
		m.guests[device] = &machineGuest{generation: 1, image: machinewire.MachineImageState{Generation: "gen-1", BaseVersion: Base, SystemBytes: 1 << 30, HomeBytes: 1 << 30}}
	}
	m.mu.Unlock()
	runner := m.takeRunner(device)
	defer func() { m.putRunner(device, runner) }()
	if (runner != nil && runner.Running()) || script.SilentWake {
		return nil
	}
	backend := params.Boot.BackendURL.String()
	token := params.Boot.DeviceToken.Expose()
	if runner != nil {
		return runner.StartAgainWithToken(ctx, backend, token)
	}
	env := host.SpawnEnv{Mode: host.Inherit}
	if script.CloudEnv != nil {
		env.Mode = host.Exactly
		env.Values = make(map[string]*string)
		for key, value := range script.CloudEnv {
			value := value
			env.Values[key] = &value
		}
	}
	if script.Artifacts != nil {
		if env.Mode == host.Inherit {
			env.Mode = host.Overlay
			env.Values = make(map[string]*string)
		}
		env.Values["DEMI_ARTIFACTS"] = script.Artifacts
	}
	var err error
	runner, err = m.startRunner(ctx, backend, remotehosttest.RunnerProcessOptions{Name: "cloud", Env: env, Token: &token, Managed: true})
	return err
}
