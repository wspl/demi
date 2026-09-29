package backend

import (
	"bufio"
	"context"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"log/slog"
	"net"
	"strconv"
	"sync"

	"github.com/wspl/demi/go/machinesproto"
	"github.com/wspl/demi/go/runnerproto"
)

// deathQueue is how many death events wait for the backend's router.
const deathQueue = 256

// notConnected is why a call over the live connection found none.
const notConnected = "no connection is open"

// MachinesErrorKind says why a call to the machine manager has no result.
type MachinesErrorKind int

const (
	// MachinesUnavailable: the manager could not be reached, or the
	// connection dropped before it answered, so whether the operation ran
	// is not known.
	MachinesUnavailable MachinesErrorKind = iota + 1
	// MachinesFailed: the manager ran the operation and it failed.
	MachinesFailed
	// MachinesResult: the manager answered with a result the operation
	// does not have.
	MachinesResult
)

// MachinesError is why a call to the machine manager has no result.
type MachinesError struct {
	Kind      MachinesErrorKind
	Operation string
	Reason    string
}

func (e *MachinesError) Error() string {
	switch e.Kind {
	case MachinesUnavailable:
		return "Machine manager unavailable during " + e.Operation + ": " + e.Reason
	case MachinesResult:
		return "the machine manager answered " + e.Operation + " with an unexpected result: " + e.Reason
	}
	return e.Reason
}

// MachinesClient is the backend's end of the machine manager's socket
// (managed-hosts.md § Control and ownership): one connection, opened by the
// first call and opened again by the next call after it drops. Requests
// carry ids the client counts, and replies are matched to them in whatever
// order they come. When the connection drops, every call in flight fails
// as unavailable: losing the socket does not establish that an operation
// failed, so the Cloud's lifecycle asks the manager before it retries. A
// death event goes to the one channel the backend routes to the device's
// owner. It is safe for use by any goroutine.
type MachinesClient struct {
	socket string
	deaths chan machinesproto.DeviceID
	// mu guards the connection, the calls in flight and the id counter; a
	// call that opens the connection holds it while it dials, so later
	// calls wait for that connection. It is never held while a request is
	// written, so the reader can always answer.
	mu      sync.Mutex
	conn    *machinesConnection
	pending map[string]pendingMachineCall
	nextID  uint64
	// writing orders the requests' writes.
	writing sync.Mutex
}

// machinesConnection is one connection to the manager; done closes when it
// is dropped, which ends its reader.
type machinesConnection struct {
	conn net.Conn
	done chan struct{}
}

type pendingMachineCall struct {
	operation string
	answer    chan machineAnswer
}

type machineAnswer struct {
	result jsontext.Value
	err    error
}

// NewMachinesClient is a client of the socket at socket, which dials
// nothing yet, and the channel of the manager's death events: the device
// of each sandbox that exited without being asked to stop.
func NewMachinesClient(socket string) (*MachinesClient, <-chan machinesproto.DeviceID) {
	deaths := make(chan machinesproto.DeviceID, deathQueue)
	return &MachinesClient{socket: socket, deaths: deaths, pending: map[string]pendingMachineCall{}, nextID: 1}, deaths
}

// Reconcile asks the manager to reconcile its sandboxes with its records.
func (c *MachinesClient) Reconcile(ctx context.Context) error {
	return c.none(ctx, machinesproto.ReconcileParams{}, false)
}

// CurrentBaseVersion is the base the manager boots new system layers from.
func (c *MachinesClient) CurrentBaseVersion(ctx context.Context) (machinesproto.BaseVersion, error) {
	call := machinesproto.CurrentBaseVersionParams{}
	result, err := c.call(ctx, call, false)
	if err != nil {
		return "", err
	}
	var text string
	if err := json.Unmarshal(result, &text); err != nil {
		return "", unexpected(call, err)
	}
	version, err := machinesproto.ParseBaseVersion(text)
	if err != nil {
		return "", unexpected(call, err)
	}
	return version, nil
}

// ImageState is the device's committed generation, or nil when it has
// none.
func (c *MachinesClient) ImageState(ctx context.Context, device machinesproto.DeviceID) (*machinesproto.ImageState, error) {
	call := machinesproto.ImageStateParams{DeviceID: string(device)}
	result, err := c.call(ctx, call, false)
	if err != nil {
		return nil, err
	}
	state, err := machinesproto.DecodeImageStateResult(result)
	if err != nil {
		return nil, unexpected(call, err)
	}
	return state, nil
}

// RuntimeState says whether the manager runs a sandbox for the device.
func (c *MachinesClient) RuntimeState(ctx context.Context, device machinesproto.DeviceID) (machinesproto.RuntimeState, error) {
	call := machinesproto.RuntimeStateParams{DeviceID: string(device)}
	result, err := c.call(ctx, call, false)
	if err != nil {
		return "", err
	}
	var state machinesproto.RuntimeState
	if err := json.Unmarshal(result, &state); err != nil {
		return "", unexpected(call, err)
	}
	if state != machinesproto.Running && state != machinesproto.Stopped {
		return "", unexpected(call, errors.New("unknown runtime state "+strconv.Quote(string(state))))
	}
	return state, nil
}

// Wake boots the device's sandbox with boot.
func (c *MachinesClient) Wake(ctx context.Context, device machinesproto.DeviceID, boot runnerproto.ManagedBoot) error {
	return c.none(ctx, machinesproto.WakeParams{DeviceID: string(device), Boot: boot}, false)
}

// Hibernate stops the device's sandbox.
func (c *MachinesClient) Hibernate(ctx context.Context, device machinesproto.DeviceID) error {
	return c.none(ctx, machinesproto.HibernateParams{DeviceID: string(device)}, false)
}

// Checkpoint commits the device's working pair as its generation.
func (c *MachinesClient) Checkpoint(ctx context.Context, device machinesproto.DeviceID) error {
	return c.none(ctx, machinesproto.CheckpointParams{DeviceID: string(device)}, false)
}

// GrowVolume grows one of the device's volumes to bytes.
func (c *MachinesClient) GrowVolume(ctx context.Context, device machinesproto.DeviceID, volume machinesproto.Volume, bytes uint64) error {
	return c.none(ctx, machinesproto.GrowVolumeParams{DeviceID: string(device), Volume: volume, Bytes: bytes}, false)
}

// Reset replaces the device's system layer with one of baseVersion, as
// operation operationID.
func (c *MachinesClient) Reset(ctx context.Context, device machinesproto.DeviceID, operationID string, baseVersion machinesproto.BaseVersion) error {
	return c.none(ctx, machinesproto.ResetParams{DeviceID: string(device), OperationID: operationID, BaseVersion: string(baseVersion)}, false)
}

// Close reconciles the manager over the connection that is open, and
// disconnects; a client that never connected has nothing to do. The manager
// keeps running, and a later call connects again.
func (c *MachinesClient) Close(ctx context.Context) error {
	err := c.none(ctx, machinesproto.ReconcileParams{}, true)
	var refused *MachinesError
	if errors.As(err, &refused) && refused.Kind == MachinesUnavailable && refused.Reason == notConnected {
		err = nil
	}
	c.mu.Lock()
	c.drop(c.conn, "the backend disconnected")
	c.mu.Unlock()
	return err
}

// none runs call, whose result is null, connecting first unless liveOnly.
func (c *MachinesClient) none(ctx context.Context, call machinesproto.Call, liveOnly bool) error {
	result, err := c.call(ctx, call, liveOnly)
	if err != nil {
		return err
	}
	if string(result) != "null" {
		return unexpected(call, errors.New("expected null"))
	}
	return nil
}

// unexpected is a result call does not have, for reason.
func unexpected(call machinesproto.Call, reason error) error {
	return &MachinesError{Kind: MachinesResult, Operation: machinesproto.OperationName(call), Reason: reason.Error()}
}

// call sends call, connecting first unless liveOnly, and waits for its
// result or ctx. A caller that leaves reads no answer; the operation runs
// all the same.
func (c *MachinesClient) call(ctx context.Context, call machinesproto.Call, liveOnly bool) (jsontext.Value, error) {
	answer, err := c.send(ctx, call, liveOnly)
	if err != nil {
		return nil, err
	}
	select {
	case answered := <-answer:
		return answered.result, answered.err
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

// send writes call's request and answers the channel its reply comes on.
func (c *MachinesClient) send(ctx context.Context, call machinesproto.Call, liveOnly bool) (<-chan machineAnswer, error) {
	operation := machinesproto.OperationName(call)
	c.mu.Lock()
	if c.conn == nil {
		if liveOnly {
			c.mu.Unlock()
			return nil, &MachinesError{Kind: MachinesUnavailable, Operation: operation, Reason: notConnected}
		}
		conn, err := (&net.Dialer{}).DialContext(ctx, "unix", c.socket)
		if err != nil {
			c.mu.Unlock()
			return nil, &MachinesError{Kind: MachinesUnavailable, Operation: operation, Reason: err.Error()}
		}
		c.conn = &machinesConnection{conn: conn, done: make(chan struct{})}
		go c.read(c.conn)
	}
	connection := c.conn
	id := strconv.FormatUint(c.nextID, 10)
	c.nextID++
	line, err := machinesproto.EncodeRequest(machinesproto.Request{ID: id, Call: call})
	if err != nil {
		c.mu.Unlock()
		return nil, err
	}
	// The reader never waits to answer: each call has room for its one.
	answer := make(chan machineAnswer, 1)
	c.pending[id] = pendingMachineCall{operation: operation, answer: answer}
	c.mu.Unlock()
	c.writing.Lock()
	_, err = connection.conn.Write(line)
	c.writing.Unlock()
	if err != nil {
		// A dropped connection fails the call with the other calls.
		c.mu.Lock()
		c.drop(connection, err.Error())
		c.mu.Unlock()
	}
	return answer, nil
}

// read reads the connection's lines until it ends, then drops it.
func (c *MachinesClient) read(connection *machinesConnection) {
	lines := bufio.NewScanner(connection.conn)
	lines.Buffer(make([]byte, 0, 64*1024), machinesproto.MaxLineBytes)
	for lines.Scan() {
		if len(lines.Bytes()) > 0 {
			c.receive(connection, lines.Bytes())
		}
	}
	reason := "the machine manager closed the connection"
	if err := lines.Err(); err != nil {
		reason = err.Error()
	}
	c.mu.Lock()
	c.drop(connection, reason)
	c.mu.Unlock()
}

// receive answers the call a reply names, or routes a death.
func (c *MachinesClient) receive(connection *machinesConnection, line []byte) {
	response, err := machinesproto.DecodeResponse(line)
	if err != nil {
		// The manager's other replies are still readable.
		slog.Warn("an unreadable line from the machine manager was dropped", "error", err)
		return
	}
	var id string
	var result jsontext.Value
	var failure *string
	switch reply := response.(type) {
	case machinesproto.Death:
		device, err := machinesproto.ParseDeviceID(reply.DeviceID)
		if err != nil {
			slog.Warn("the machine manager reported the death of a device it cannot name", "error", err)
			return
		}
		select {
		case c.deaths <- device:
		case <-connection.done:
			// The backend let go of this connection; it learns of the
			// device's state from the manager when it asks next.
		}
		return
	case machinesproto.OK:
		id, result = reply.ID, reply.Result
	case machinesproto.Failure:
		id, failure = reply.ID, &reply.Message
	}
	c.mu.Lock()
	pending, ok := c.pending[id]
	delete(c.pending, id)
	c.mu.Unlock()
	if !ok {
		slog.Warn("the machine manager answered a request this backend did not send", "id", id)
		return
	}
	if failure != nil {
		pending.answer <- machineAnswer{err: &MachinesError{Kind: MachinesFailed, Operation: pending.operation, Reason: *failure}}
		return
	}
	pending.answer <- machineAnswer{result: result}
}

// drop lets go of connection, if it is still the one in use: every call in
// flight fails, and the next call connects again. The caller holds mu.
func (c *MachinesClient) drop(connection *machinesConnection, reason string) {
	if connection == nil || c.conn != connection {
		return
	}
	c.conn = nil
	close(connection.done)
	// Closing a connection the manager closed already fails harmlessly.
	_ = connection.conn.Close()
	if len(c.pending) > 0 {
		slog.Warn("the machine manager's connection dropped", "calls", len(c.pending), "reason", reason)
	}
	for id, pending := range c.pending {
		pending.answer <- machineAnswer{err: &MachinesError{Kind: MachinesUnavailable, Operation: pending.operation, Reason: reason}}
		delete(c.pending, id)
	}
}
