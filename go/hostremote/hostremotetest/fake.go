package hostremotetest

import (
	"context"
	"errors"
	"io"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/go/hostremote"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
)

type shardRequest struct {
	ctx    context.Context
	fn     func()
	result chan error
}

// TestDevice owns an actual serial executor, so its tests expose waits in shard
// closures rather than concealing them with a mutex around callbacks.
type TestDevice struct {
	Pipes    *hostremote.Pipes
	Policy   hostremote.LinkPolicy
	Execute  hostremote.Executor
	requests chan shardRequest
	stopped  chan struct{}
	stop     chan struct{}
	state    hostremote.DeviceLink
	changed  chan struct{}
	links    []*TestLink
	close    sync.Once
}

func NewTestDevice(t testing.TB, policy hostremote.LinkPolicy) *TestDevice {
	t.Helper()
	if policy == nil {
		policy = NewCommandPolicy(nil)
	}
	d := &TestDevice{Pipes: hostremote.NewPipes(hostremote.Arrival), Policy: policy, requests: make(chan shardRequest), stopped: make(chan struct{}), stop: make(chan struct{}), changed: make(chan struct{})}
	go func() {
		defer close(d.stopped)
		for {
			var request shardRequest
			select {
			case <-d.stop:
				return
			case request = <-d.requests:
			}
			err := request.ctx.Err()
			if err == nil {
				request.fn()
			}
			request.result <- err
		}
	}()
	d.Execute = func(ctx context.Context, fn func()) error {
		reply := make(chan error, 1)
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-d.stopped:
			return errors.New("test shard stopped")
		case d.requests <- shardRequest{ctx, fn, reply}:
		}
		return <-reply
	}
	t.Cleanup(d.Close)
	return d
}
func (d *TestDevice) Host(cwd string, admission hostremote.Admission) *hostremote.RemoteHost {
	return hostremote.NewRemoteHost(shell.HostKey(TestDeviceID+":"+cwd), cwd, d.Execute, func() hostremote.DeviceLink { return d.state }, admission)
}
func (d *TestDevice) adopt(transport hostremote.Transport, identity shell.HostIdentity, ping time.Duration, tap chan<- runnerproto.Outbound) *TestLink {
	link, driver := hostremote.NewLink(hostremote.LinkOptions{Device: TestDeviceID, Identity: identity, Pipes: d.Pipes, Policy: d.Policy, Ping: ping})
	driver.Tap = tap
	test := &TestLink{Link: link, ended: make(chan struct{})}
	_ = d.Execute(context.Background(), func() {
		d.state = hostremote.DeviceLink{Link: link}
		d.links = append(d.links, test)
		close(d.changed)
		d.changed = make(chan struct{})
	})
	go func() {
		test.end = driver.Serve(context.Background(), transport)
		_ = d.Execute(context.Background(), func() {
			if d.state.Link == link {
				d.state = hostremote.DeviceLink{Last: &identity}
				close(d.changed)
				d.changed = make(chan struct{})
			}
		})
		close(test.ended)
	}()
	return test
}
func (d *TestDevice) Connect(ping time.Duration) *TestLink {
	transport := &channelTransport{received: make(chan []byte, 8), sent: make(chan []byte, hostremote.OutboundFrames), done: make(chan struct{})}
	test := d.adopt(transport, shell.HostIdentity{UID: 501, GID: 20, Hostname: "test", HomeDir: "/work"}, ping, nil)
	test.transport = transport
	return test
}
func (d *TestDevice) Online(ctx context.Context) (*hostremote.Link, error) {
	for {
		var link *hostremote.Link
		var changed <-chan struct{}
		if err := d.Execute(ctx, func() {
			link = d.state.Link
			changed = d.changed
		}); err != nil {
			return nil, err
		}
		if link != nil && !link.IsClosed() {
			return link, nil
		}
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-changed:
		}
	}
}
func (d *TestDevice) Close() {
	d.close.Do(func() {
		var links []*TestLink
		_ = d.Execute(context.Background(), func() { links = append(links, d.links...) })
		for _, link := range links {
			link.Link.Disconnect("test over")
		}
		for _, link := range links {
			<-link.ended
		}
		d.Pipes.Close()
		close(d.stop)
		<-d.stopped
	})
}

type TestLink struct {
	Link      *hostremote.Link
	transport *channelTransport
	ended     chan struct{}
	end       hostremote.LinkEnd
}

func (t *TestLink) Next(ctx context.Context) (runnerproto.Inbound, error) {
	select {
	case frame := <-t.transport.sent:
		return runnerproto.DecodeInboundMsgpack(frame)
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-t.ended:
		return nil, io.EOF
	}
}
func (t *TestLink) TryNext() (runnerproto.Inbound, bool, error) {
	select {
	case frame := <-t.transport.sent:
		value, err := runnerproto.DecodeInboundMsgpack(frame)
		return value, true, err
	default:
		return nil, false, nil
	}
}
func (t *TestLink) Send(ctx context.Context, message runnerproto.Outbound) error {
	frame, err := runnerproto.EncodeOutboundMsgpack(message)
	if err != nil {
		return err
	}
	return t.SendFrame(ctx, frame)
}
func (t *TestLink) SendFrame(ctx context.Context, frame []byte) error {
	select {
	case t.transport.received <- frame:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	case <-t.ended:
		return io.EOF
	}
}
func (t *TestLink) Ended(ctx context.Context) (hostremote.LinkEnd, error) {
	select {
	case <-ctx.Done():
		return hostremote.LinkEnd{}, ctx.Err()
	case <-t.ended:
		return t.end, nil
	}
}
func (t *TestLink) Close() hostremote.LinkEnd {
	t.transport.Close()
	<-t.ended
	return t.end
}

type channelTransport struct {
	received, sent chan []byte
	done           chan struct{}
	once           sync.Once
}

func (t *channelTransport) Read(ctx context.Context) ([]byte, error) {
	select {
	case frame := <-t.received:
		return frame, nil
	case <-t.done:
		return nil, io.EOF
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}
func (t *channelTransport) Write(ctx context.Context, frame []byte) error {
	select {
	case t.sent <- frame:
		return nil
	case <-t.done:
		return io.EOF
	case <-ctx.Done():
		return ctx.Err()
	}
}
func (t *channelTransport) Close() error {
	t.once.Do(func() { close(t.done) })
	return nil
}
