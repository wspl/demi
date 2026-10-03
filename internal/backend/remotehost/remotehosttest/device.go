package remotehosttest

import (
	"context"
	"errors"
	"io"
	"slices"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/host/hosttest"
	"github.com/wspl/demi/internal/runnerwire"
)

// TestDeviceID names the device a test connection serves.
const TestDeviceID = "test-device"

// CommandPolicy runs all calls in one command set with each node's storage in memory.
type CommandPolicy struct {
	commands  *host.CommandSet
	mu        sync.Mutex // Protects node storage registration and sequence allocation.
	storage   map[string]*hosttest.MemoryStorage
	sequences map[sequenceKey]uint64
}

// NewCommandPolicy constructs the policy for commands.
func NewCommandPolicy(commands *host.CommandSet) *CommandPolicy {
	if commands == nil {
		commands = &host.CommandSet{}
	}
	return &CommandPolicy{
		commands:  commands,
		storage:   make(map[string]*hosttest.MemoryStorage),
		sequences: make(map[sequenceKey]uint64),
	}
}

// NodeStorage returns the command storage of node.
func (p *CommandPolicy) NodeStorage(node string) *hosttest.MemoryStorage {
	p.mu.Lock()
	defer p.mu.Unlock()
	storage := p.storage[node]
	if storage == nil {
		storage = &hosttest.MemoryStorage{}
		p.storage[node] = storage
	}
	return storage
}

// AdmitCall admits every test call.
func (p *CommandPolicy) AdmitCall(_ remotehost.JobOrigin) error {
	return nil
}

// Dispatch runs invocation in the test command set.
func (p *CommandPolicy) Dispatch(
	ctx context.Context,
	_ remotehost.JobOrigin,
	invocation host.RPCInvocation,
	port host.RPCPort,
) (uint8, error) {
	return p.commands.Dispatch(ctx, invocation, port)
}

// Storage operates on the caller node's storage while the call lives.
func (p *CommandPolicy) Storage(
	ctx context.Context,
	job remotehost.JobOrigin,
	op host.StorageOp,
) (host.StorageReply, error) {
	if job.Caller == nil {
		return nil, &host.PortError{Kind: host.StorageRefused, Message: "the job has no command storage"}
	}
	storage := p.NodeStorage(string(job.Caller.Node))
	if ctx.Err() != nil {
		return nil, &host.PortError{Kind: host.PortEnded, Message: "the call was stopped"}
	}
	return storage.Apply(op), nil
}

// GrowVolume refuses growth, which is unavailable in this fixture.
func (p *CommandPolicy) GrowVolume(_ context.Context, _ runnerwire.VolumeName, _ uint64) error {
	return errors.New("volume growth is not available")
}

// ReserveNumbers counts each conversation's sequence from one.
func (p *CommandPolicy) ReserveNumbers(
	_ context.Context,
	conversation string,
	sequence commandwire.ServiceSequence,
	count uint32,
) (uint64, error) {
	p.mu.Lock()
	defer p.mu.Unlock()
	key := sequenceKey{conversation: conversation, sequence: sequence}
	first, ok := p.sequences[key]
	if !ok {
		first = 1
	}
	p.sequences[key] = first + uint64(count)
	return first, nil
}

// TestDevice is an in-process fake runner whose connections are TestLinks.
// Construction registers cleanup for all links and the pipe broker.
type TestDevice struct {
	mu       sync.Mutex // Protects current connection and owned link registration.
	current  remotehost.DeviceLink
	links    []*TestLink
	closed   bool
	identity host.Identity
	pipes    *remotehost.Pipes
	policy   remotehost.LinkPolicy
}

// NewTestDevice creates a device with policy and registers test cleanup.
func NewTestDevice(t testing.TB, policy remotehost.LinkPolicy) *TestDevice {
	d := &TestDevice{
		policy:   policy,
		pipes:    remotehost.NewPipes(remotehost.Arrival),
		identity: host.Identity{UID: 501, GID: 20, Hostname: "test", HomeDir: "/work"},
	}
	t.Cleanup(func() {
		if err := d.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	return d
}

// Pipes returns the device's pipe broker.
func (d *TestDevice) Pipes() *remotehost.Pipes {
	return d.pipes
}

// Host constructs a Host on this device with cwd and admission.
func (d *TestDevice) Host(cwd string, admission remotehost.Admission) *remotehost.Host {
	return remotehost.NewHost(host.Key(TestDeviceID+":"+cwd), cwd, func() remotehost.DeviceLink {
		d.mu.Lock()
		defer d.mu.Unlock()
		return d.current
	}, admission)
}

// Connect connects a fake runner. Nil ping disables liveness.
func (d *TestDevice) Connect(ping *time.Duration) *TestLink {
	link, driver := remotehost.NewLink(
		remotehost.LinkOptions{
			Device:   TestDeviceID,
			Identity: d.identity,
			Pipes:    d.pipes,
			Policy:   d.policy,
			Ping:     ping,
		},
	)
	ctx, cancel := context.WithCancel(context.Background())
	connection := &TestLink{
		link:     link,
		incoming: make(chan []byte, 8),
		outgoing: make(chan []byte, remotehost.OutboundFrames),
		ctx:      ctx,
		cancel:   cancel,
		done:     make(chan struct{}),
	}
	d.mu.Lock()
	d.current = remotehost.DeviceLink{Link: link}
	d.links = append(d.links, connection)
	closed := d.closed
	d.mu.Unlock()
	if closed {
		cancel()
	}
	serve := func() {
		connection.end = driver.Serve(
			context.Background(),
			testFrames{frames: connection.incoming, gone: ctx.Done()},
			testFrames{frames: connection.outgoing},
		)
		d.mu.Lock()
		if d.current.Link == link {
			d.current = remotehost.DeviceLink{Last: new(d.identity)}
		}
		d.mu.Unlock()
		cancel()
		close(connection.done)
	}
	if closed {
		serve()
	} else {
		go serve()
	}
	return connection
}

// Close closes all connections and pipes and joins their workers.
func (d *TestDevice) Close(ctx context.Context) error {
	d.mu.Lock()
	d.closed = true
	links := slices.Clone(d.links)
	d.mu.Unlock()
	var result error
	for _, link := range links {
		if _, err := link.Close(context.WithoutCancel(ctx)); err != nil {
			result = errors.Join(result, err)
		}
	}
	return errors.Join(result, d.pipes.Close(context.WithoutCancel(ctx)))
}

// TestLink drives the real Link engine with an in-process runner's frames.
type TestLink struct {
	link               *remotehost.Link
	incoming, outgoing chan []byte
	ctx                context.Context
	cancel             context.CancelFunc
	done               chan struct{}
	end                remotehost.LinkEnd
}

// Link returns the backend connection handle.
func (l *TestLink) Link() *remotehost.Link {
	return l.link
}

// Next waits for and decodes the backend's next message.
func (l *TestLink) Next(ctx context.Context) (runnerwire.Inbound, error) {
	select {
	case frame := <-l.outgoing:
		return runnerwire.DecodeInbound(frame)
	case <-l.done:
		select {
		case frame := <-l.outgoing:
			return runnerwire.DecodeInbound(frame)
		default:
			return nil, io.EOF
		}
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

// TryNext takes a queued message without waiting; false means no queued frame.
func (l *TestLink) TryNext() (runnerwire.Inbound, bool, error) {
	select {
	case frame := <-l.outgoing:
		message, err := runnerwire.DecodeInbound(frame)
		return message, true, err
	default:
		return nil, false, nil
	}
}

// Send encodes and sends a runner message to the backend.
func (l *TestLink) Send(ctx context.Context, message runnerwire.Outbound) error {
	frame, err := runnerwire.Encode(message)
	if err != nil {
		return err
	}
	return l.SendFrame(ctx, frame)
}

// SendFrame sends raw bytes, including malformed frames for refusal tests.
func (l *TestLink) SendFrame(ctx context.Context, frame []byte) error {
	select {
	case <-l.ctx.Done():
		return io.EOF
	case <-ctx.Done():
		return ctx.Err()
	case l.incoming <- slices.Clone(frame):
		return nil
	}
}

// Close disconnects the runner and joins its driver.
func (l *TestLink) Close(ctx context.Context) (remotehost.LinkEnd, error) {
	l.cancel()
	return l.Ended(ctx)
}

// Ended waits for the driver to finish and joins it.
func (l *TestLink) Ended(ctx context.Context) (remotehost.LinkEnd, error) {
	select {
	case <-l.done:
		return l.end, nil
	case <-ctx.Done():
		return remotehost.LinkEnd{}, ctx.Err()
	}
}

var _ remotehost.LinkPolicy = (*CommandPolicy)(nil)

type sequenceKey struct {
	conversation string
	sequence     commandwire.ServiceSequence
}

// testFrames carries one direction of an in-process runner connection.
type testFrames struct {
	frames chan []byte
	gone   <-chan struct{}
}

// Receive reads accepted runner frames before reporting disconnection.
func (f testFrames) Receive(ctx context.Context) ([]byte, error) {
	// A runner that closes still leaves its already accepted frames to be read.
	select {
	case frame := <-f.frames:
		return frame, nil
	default:
	}
	select {
	case <-f.gone:
		return nil, io.EOF
	case frame := <-f.frames:
		return frame, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

// Send queues a backend frame until delivery or cancellation.
func (f testFrames) Send(ctx context.Context, frame []byte) error {
	select {
	case f.frames <- frame:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}
