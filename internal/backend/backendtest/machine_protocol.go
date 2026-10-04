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
	"github.com/wspl/demi/internal/machinemanagerproto"
	"github.com/wspl/demi/internal/webapiproto"
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
	outgoing := make(chan machinemanagerproto.MachineResponse, 64)
	writerDone := make(chan struct{})
	go func() {
		defer close(writerDone)
		defer cancel()
		defer func() {
			// Unblocks the reader; an already closed socket needs no further cleanup.
			_ = socket.Close()
		}()
		m.writeResponses(ctx, connection, outgoing)
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
		request, err := machinemanagerproto.DecodeRequest(line)
		if err != nil {
			break
		}
		requests.Go(func() {
			result, err := m.handle(m.ctx, request.Call)
			var response machinemanagerproto.MachineResponse = &machinemanagerproto.OK{ID: request.ID, Result: result}
			if err != nil {
				response = &machinemanagerproto.ErrorResponse{ID: request.ID, Message: err.Error()}
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
func (m *ScriptedManager) handle(ctx context.Context, call machinemanagerproto.Call) (json.RawMessage, error) {
	if _, ok := call.(*machinemanagerproto.Reconcile); ok {
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
	case *machinemanagerproto.Reconcile: // Handled before device admission.
	case *machinemanagerproto.CurrentBaseVersion:
		return contract.EncodeJSON(machinemanagerproto.BaseVersion(Base))
	case *machinemanagerproto.ImageState:
		m.mu.Lock()
		var image *machinemanagerproto.MachineImageState
		if guest := m.guests[call.Params.DeviceID]; guest != nil {
			snapshot := guest.image
			image = &snapshot
		}
		m.mu.Unlock()
		return contract.EncodeJSON(image)
	case *machinemanagerproto.RuntimeStateCall:
		device = call.Params.DeviceID
	case *machinemanagerproto.Wake:
		device = call.Params.DeviceID
	case *machinemanagerproto.Hibernate:
		device = call.Params.DeviceID
	case *machinemanagerproto.Checkpoint:
		device = call.Params.DeviceID
	case *machinemanagerproto.GrowVolume:
		device = call.Params.DeviceID
	case *machinemanagerproto.Reset:
		device = call.Params.DeviceID
	}
	permit, err := m.turns.Acquire(ctx, device)
	if err != nil {
		return nil, err
	}
	defer permit.Release()
	return m.handleDevice(ctx, device, call)
}

// handleDevice executes a manager request while its caller owns device admission.
func (m *ScriptedManager) handleDevice(
	ctx context.Context,
	device string,
	call machinemanagerproto.Call,
) (json.RawMessage, error) {
	switch call := call.(type) {
	case *machinemanagerproto.Reconcile, *machinemanagerproto.CurrentBaseVersion, *machinemanagerproto.ImageState:
		return nil, errors.New("manager operation reached device admission unexpectedly")
	case *machinemanagerproto.RuntimeStateCall:
		state := machinemanagerproto.RuntimeStateStopped
		if m.Running(webapiproto.DeviceID(device)) {
			state = machinemanagerproto.RuntimeStateRunning
		}
		return contract.EncodeJSON(state)
	case *machinemanagerproto.Wake:
		m.record("wake:" + device)
		return json.RawMessage("null"), m.wake(ctx, device, call.Params)
	case *machinemanagerproto.Hibernate:
		m.record("hibernate:" + device)
		m.mu.Lock()
		failure := m.script.FailHibernate
		m.script.FailHibernate = nil
		m.mu.Unlock()
		if failure != nil {
			return nil, errors.New(*failure)
		}
		return json.RawMessage("null"), m.stopRunner(ctx, device)
	case *machinemanagerproto.Checkpoint:
		m.record("checkpoint:" + device)
	case *machinemanagerproto.GrowVolume:
		p := call.Params
		m.record(fmt.Sprintf("grow:%s:%s:%d", device, p.Volume, p.Bytes))
		m.mu.Lock()
		defer m.mu.Unlock()
		guest := m.guests[device]
		if guest == nil {
			return nil, errors.New("the device has no storage")
		}
		if p.Volume == machinemanagerproto.VolumeSystem {
			guest.image.SystemBytes = max(guest.image.SystemBytes, p.Bytes)
		} else {
			guest.image.HomeBytes = max(guest.image.HomeBytes, p.Bytes)
		}
	case *machinemanagerproto.Reset:
		if err := m.reset(ctx, device, call.Params); err != nil {
			return nil, err
		}
	}
	return json.RawMessage("null"), nil
}

// wake starts or resumes the device's existing runner without replacing its home.
func (m *ScriptedManager) wake(ctx context.Context, device string, params machinemanagerproto.WakeParams) error {
	m.mu.Lock()
	script := m.script
	if m.guests[device] == nil {
		m.guests[device] = &machineGuest{
			generation: 1,
			image: machinemanagerproto.MachineImageState{
				Generation:  "gen-1",
				BaseVersion: Base,
				SystemBytes: 1 << 30,
				HomeBytes:   1 << 30,
			},
		}
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
	runner, err = m.startRunner(
		ctx,
		backend,
		remotehosttest.RunnerProcessOptions{Name: "cloud", Env: env, Token: token, Managed: true},
	)
	return err
}

// writeResponses serializes replies and death events onto one manager connection.
func (m *ScriptedManager) writeResponses(
	ctx context.Context,
	connection *machineConnection,
	outgoing <-chan machinemanagerproto.MachineResponse,
) {
	for {
		var message machinemanagerproto.MachineResponse
		select {
		case <-ctx.Done():
			return
		case device := <-connection.deaths:
			message = &machinemanagerproto.Death{DeviceID: device}
		case reply, ok := <-outgoing:
			if !ok {
				return
			}
			message = reply
		}
		line, err := machinemanagerproto.EncodeLine(message)
		if err != nil {
			return
		}
		if _, err := connection.socket.Write(line); err != nil {
			return
		}
	}
}

// reset clears transient runner state and advances the scripted disk generation.
func (m *ScriptedManager) reset(ctx context.Context, device string, params machinemanagerproto.ResetParams) error {
	m.record(fmt.Sprintf("reset:%s:%s:%s", device, params.OperationID, params.BaseVersion))
	if err := m.resets.Pass(ctx, "reset"); err != nil {
		return err
	}
	m.mu.Lock()
	failure := m.script.FailReset
	m.script.FailReset = nil
	m.mu.Unlock()
	if failure != nil {
		return errors.New(*failure)
	}
	if err := m.stopRunner(ctx, device); err != nil {
		return err
	}
	runner := m.takeRunner(device)
	if runner != nil {
		err := runner.ClearState(ctx)
		m.putRunner(device, runner)
		if err != nil {
			return err
		}
	}
	m.mu.Lock()
	if guest := m.guests[device]; guest != nil {
		guest.generation++
		guest.image.Generation = machinemanagerproto.GenerationID(fmt.Sprintf("gen-%d", guest.generation))
		guest.image.ResetID = &params.OperationID
	}
	m.mu.Unlock()
	return nil
}
