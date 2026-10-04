package remotehost

import (
	"context"
	"crypto/rand"
	"errors"
	"fmt"
	"io"
	"sync"
	"time"
	"weak"

	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/types"
)

// PingInterval is how often the backend asks a runner whether it is there.
const PingInterval = 30 * time.Second

// OutboundFrames is the frames the backend queues before a sender waits.
const OutboundFrames = 64

// LinkPolicy supplies the product's decisions for a connection's jobs and calls.
// Implementations must support concurrent calls without holding locks across IO.
type LinkPolicy interface {
	// AdmitCall refuses a call before any pipe is minted.
	AdmitCall(job JobOrigin) error
	// Dispatch runs the RPC invocation from job.
	Dispatch(context.Context, JobOrigin, host.RPCInvocation, host.RPCPort) (uint8, error)
	// Storage operates on the job's command storage; writes commit only while the context lives.
	Storage(context.Context, JobOrigin, host.StorageOp) (host.StorageReply, error)
	// GrowVolume handles a managed guest's request for a larger volume.
	GrowVolume(context.Context, runnerproto.VolumeName, uint64) error
	// ReserveNumbers reserves count numbers of the conversation's sequence.
	ReserveNumbers(context.Context, string, commandproto.ServiceSequence, uint32) (uint64, error)
}

// JobOrigin records whose a job is when it starts.
type JobOrigin struct {
	// Host identifies the admitted Host.
	Host host.Key
	// Context identifies the conversation and command locale.
	Context commandproto.Context
	// Caller identifies the node that owns command callbacks.
	Caller *host.JobCaller
}

// LinkOptions describes a runner connection. Zero Ping disables liveness checks.
type LinkOptions struct {
	// Device identifies the connected device.
	Device string
	// Identity is the account reported by the runner.
	Identity host.Identity
	// Pipes is the broker serving this connection.
	Pipes *Pipes
	// Policy supplies callback admission and execution.
	Policy LinkPolicy
	// Ping is the liveness interval; zero disables probes.
	Ping time.Duration
}

// LinkEndKind identifies why a connection ended.
type LinkEndKind uint8

const (
	// LinkClosed means the runner's side went away.
	LinkClosed LinkEndKind = iota
	// LinkRefused means the runner broke the protocol.
	LinkRefused
	// LinkDisconnected means the backend ended the connection.
	LinkDisconnected
)

// LinkEnd is the terminal reason for a connection.
type LinkEnd struct {
	// Kind identifies why the connection ended.
	Kind LinkEndKind
	// Reason describes the connection end.
	Reason string
}

// Link is one runner connection. Pointer identity identifies the connection.
// Its driver owns and joins its workers before Serve returns.
type Link struct {
	device             string
	identity           host.Identity
	pipes              *Pipes
	policy             LinkPolicy
	ctx                context.Context
	cancel             context.CancelFunc
	outbound           chan []byte
	mu                 sync.Mutex // Protects request registrations, liveness and immutable snapshots.
	waiting            map[string]*replyWait
	installs           []runnerproto.Install
	installsChanged    chan struct{}
	liveness           pingState
	disconnect         *string
	endReason          string
	pongJobs           uint64
	workers            sync.WaitGroup
	services           map[string]*ServiceStream
	jobs               map[string]*Job
	spawns             map[string]*runnerProcess
	calls              map[string]*relayCall
	numbers, artifacts map[string]struct{}
	jobTurn            gates.Serial
	manifest           string // Protected by jobTurn, never the state mutex.
}

// WeakLink identifies a connection without retaining it.
type WeakLink struct {
	pointer weak.Pointer[Link]
}

// Is reports whether link is the connection this names.
func (w WeakLink) Is(link *Link) bool {
	return w.pointer.Value() == link && link != nil
}

// NewLink creates a connection and its single driver.
func NewLink(options LinkOptions) (*Link, *LinkDriver) {
	ctx, cancel := context.WithCancel(context.Background())
	l := &Link{
		device:          options.Device,
		identity:        options.Identity,
		pipes:           options.Pipes,
		policy:          options.Policy,
		ctx:             ctx,
		cancel:          cancel,
		outbound:        make(chan []byte, OutboundFrames),
		waiting:         make(map[string]*replyWait),
		services:        make(map[string]*ServiceStream),
		jobs:            make(map[string]*Job),
		spawns:          make(map[string]*runnerProcess),
		calls:           make(map[string]*relayCall),
		numbers:         make(map[string]struct{}),
		artifacts:       make(map[string]struct{}),
		installsChanged: make(chan struct{}),
	}
	return l, &LinkDriver{link: l, ping: options.Ping}
}

// Weak returns a non-retaining connection identity.
func (l *Link) Weak() WeakLink {
	return WeakLink{pointer: weak.Make(l)}
}

// Device returns the device whose pipe ends this connection uses.
func (l *Link) Device() string {
	return l.device
}

// Identity returns the account reported by the runner.
func (l *Link) Identity() host.Identity {
	return l.identity
}

// Pipes returns the connection's pipe broker.
func (l *Link) Pipes() *Pipes {
	return l.pipes
}

// Installs returns an immutable snapshot of installation progress.
func (l *Link) Installs() []runnerproto.Install {
	installs, _ := l.WatchInstalls()
	return installs
}

// WatchInstalls returns progress and a channel closed on the next change.
// Reload after notification; no subscription needs releasing.
func (l *Link) WatchInstalls() ([]runnerproto.Install, <-chan struct{}) {
	l.mu.Lock()
	defer l.mu.Unlock()
	// A runner with no installs reports an empty list, never none: the device
	// answers carry it as a required array.
	installs := make([]runnerproto.Install, len(l.installs))
	copy(installs, l.installs)
	return installs, l.installsChanged
}

// IsClosed reports whether the connection ended.
func (l *Link) IsClosed() bool {
	return l.ctx.Err() != nil
}

// Disconnect ends the connection for reason without waiting for its driver.
func (l *Link) Disconnect(reason string) {
	l.mu.Lock()
	if l.disconnect == nil {
		l.disconnect = new(reason)
	}
	l.mu.Unlock()
	l.cancel()
}

// PauseLiveness suspends ping expiry while the device is paused.
func (l *Link) PauseLiveness() {
	l.mu.Lock()
	l.liveness = pingPaused
	l.mu.Unlock()
}

// ResumeLiveness resumes ping expiry after a device pause.
func (l *Link) ResumeLiveness() {
	l.mu.Lock()
	l.liveness = pingIdle
	l.mu.Unlock()
}

// RunningJobs counts jobs still running on this connection.
func (l *Link) RunningJobs() uint64 {
	l.mu.Lock()
	defer l.mu.Unlock()
	count := uint64(len(l.jobs))
	for _, process := range l.spawns {
		if !process.retained {
			count++
		}
	}
	return max(count, l.pongJobs)
}

// Sync waits for the runner's synchronization reply.
func (l *Link) Sync(ctx context.Context) error {
	_, err := l.call(ctx, "Sync", func(id string) runnerproto.Inbound {
		return &runnerproto.Sync{ID: id}
	})
	return err
}

// ReleaseConversation retires the conversation's resident services.
func (l *Link) ReleaseConversation(ctx context.Context, conversation string) error {
	releaseCtx, cancel := context.WithTimeout(ctx, 360*time.Second)
	defer cancel()
	_, err := l.call(releaseCtx, "Release", func(id string) runnerproto.Inbound {
		return &runnerproto.ConversationRelease{ID: id, ConversationID: conversation}
	})
	if errors.Is(err, context.DeadlineExceeded) && ctx.Err() == nil {
		return &host.Error{Kind: host.Interrupted, Message: "Conversation release timed out"}
	}
	return err
}

// JobOutput holds one output view and its offset in the stream.
type JobOutput struct {
	// Stream identifies stdout or stderr.
	Stream types.StreamKind
	// Offset is the chunk start in that stream.
	Offset uint64
	// Bytes contains the output chunk.
	Bytes []byte
}

// JobEnd holds the job's terminal status, directory, output lengths and edits.
type JobEnd struct {
	// Status is the process completion reported by the runner.
	Status host.ProcessEnd
	// CWD is the final shell directory, when reported.
	CWD *string
	// Output contains the final stream lengths, when reported.
	Output *runnerproto.OutputLengths
	// Files lists tracked file changes.
	Files []runnerproto.JobFileChange
	// FilesTruncated reports whether tracking omitted additional file changes.
	FilesTruncated bool
}

// FrameSource supplies complete runner frames. EOF ends the connection.
// Receive must unblock on context cancellation; the caller owns the transport.
type FrameSource interface {
	Receive(context.Context) ([]byte, error)
}

// FrameSink accepts complete backend frames and unblocks on context cancellation.
// The caller owns the transport; LinkDriver does not own a socket.
type FrameSink interface {
	Send(context.Context, []byte) error
}

// LinkDriver routes frames and owns connection workers for one Serve call.
type LinkDriver struct {
	link *Link
	ping time.Duration
	tap  chan<- runnerproto.Outbound
}

// Tap copies decoded runner messages for wire audits; a full channel loses the copy.
// Set it before Serve. The caller owns the channel and closes it only after Serve.
func (d *LinkDriver) Tap(tap chan<- runnerproto.Outbound) {
	d.tap = tap
}

// Serve routes incoming replies, sends queued frames and pings until the connection ends.
// Cancellation ends all in-flight work; Serve joins all workers before returning.
func (d *LinkDriver) Serve(ctx context.Context, incoming FrameSource, outgoing FrameSink) LinkEnd {
	l := d.link
	serveCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	ends := make(chan LinkEnd, 2)
	var ioWorkers sync.WaitGroup
	ioWorkers.Add(2)
	go func() {
		defer ioWorkers.Done()
		ends <- d.readFrames(serveCtx, incoming)
		cancel()
	}()
	go func() {
		defer ioWorkers.Done()
		ends <- d.writeFrames(serveCtx, outgoing)
		cancel()
	}()
	var ticks <-chan time.Time
	if d.ping > 0 {
		ticker := time.NewTicker(d.ping)
		defer ticker.Stop()
		ticks = ticker.C
	}
	var end LinkEnd
loop:
	for {
		select {
		case end = <-ends:
			break loop
		case <-ctx.Done():
			end = LinkEnd{Kind: LinkDisconnected, Reason: "runner disconnected"}
			break loop
		case <-l.ctx.Done():
			l.mu.Lock()
			reason := "runner disconnected"
			if l.disconnect != nil {
				reason = *l.disconnect
			}
			l.mu.Unlock()
			end = LinkEnd{Kind: LinkDisconnected, Reason: reason}
			break loop
		case <-ticks:
			if pingEnd, ended := l.ping(serveCtx); ended {
				end = pingEnd
				break loop
			}
		}
	}
	reason := "runner disconnected"
	if end.Kind == LinkDisconnected {
		reason = end.Reason
	}
	cancel()
	l.teardown(reason)
	ioWorkers.Wait()
	l.workers.Wait()
	return end
}

type pingState uint8

const (
	pingIdle pingState = iota
	pingWaiting
	pingPaused
)

// replyWait binds one runner response to the operation that requested it.
type replyWait struct {
	expected string
	ready    chan reply
}
type reply struct {
	message runnerproto.Outbound
	err     error
}

// offline reports the established connection-end reason.
func (l *Link) offline() error {
	l.mu.Lock()
	reason := l.endReason
	if reason == "" && l.disconnect != nil {
		reason = *l.disconnect
	}
	l.mu.Unlock()
	if reason == "" {
		reason = "runner disconnected"
	}
	return &host.Error{Kind: host.Offline, Message: reason}
}

// send encodes and queues a runner request without holding connection state.
func (l *Link) send(ctx context.Context, message runnerproto.Inbound) error {
	if l.IsClosed() {
		return l.offline()
	}
	frame, err := runnerproto.Encode(message)
	if err != nil {
		return &host.Error{Kind: host.Protocol, Message: err.Error()}
	}
	if len(frame) > runnerproto.MaxMessageBytes {
		return &host.Error{
			Kind: host.TooLarge,
			Message: fmt.Sprintf(
				"the request is %d bytes, over the %d-byte message limit",
				len(frame),
				runnerproto.MaxMessageBytes,
			),
		}
	}
	select {
	case <-l.ctx.Done():
		return l.offline()
	case <-ctx.Done():
		return ctx.Err()
	case l.outbound <- frame:
		return nil
	}
}

// call registers a runner reply before sending the request and forgets canceled waits.
func (l *Link) call(
	ctx context.Context,
	expected string,
	build func(string) runnerproto.Inbound,
) (runnerproto.Outbound, error) {
	return l.callID(ctx, rand.Text(), expected, build)
}

// callID registers a stream-open reply under the stream's already allocated identity.
func (l *Link) callID(
	ctx context.Context,
	id, expected string,
	build func(string) runnerproto.Inbound,
) (runnerproto.Outbound, error) {
	waiting := &replyWait{expected: expected, ready: make(chan reply, 1)}
	l.mu.Lock()
	if l.IsClosed() {
		l.mu.Unlock()
		return nil, l.offline()
	}
	l.waiting[id] = waiting
	l.mu.Unlock()
	defer func() {
		l.mu.Lock()
		delete(l.waiting, id)
		l.mu.Unlock()
	}()
	if err := l.send(ctx, build(id)); err != nil {
		return nil, err
	}
	select {
	case answer := <-waiting.ready:
		return answer.message, answer.err
	case <-l.ctx.Done():
		return nil, l.offline()
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

// answer delivers exactly one reply outside the connection mutex.
func (l *Link) answer(id, expected string, message runnerproto.Outbound, err error) {
	l.mu.Lock()
	waiting := l.waiting[id]
	delete(l.waiting, id)
	l.mu.Unlock()
	if waiting == nil {
		return
	}
	if err == nil && waiting.expected != expected {
		err = &host.Error{
			Kind:    host.Protocol,
			Message: fmt.Sprintf("the runner answered %s to a %s request", expected, waiting.expected),
		}
	}
	waiting.ready <- reply{message: message, err: err}
}

// readFrames validates external runner bytes before routing them.
func (d *LinkDriver) readFrames(ctx context.Context, incoming FrameSource) LinkEnd {
	for {
		frame, err := incoming.Receive(ctx)
		if err != nil {
			reason := "runner disconnected"
			if !errors.Is(err, io.EOF) {
				reason += ": " + err.Error()
			}
			return LinkEnd{Kind: LinkClosed, Reason: reason}
		}
		message, err := runnerproto.DecodeOutbound(frame)
		if err != nil {
			return LinkEnd{Kind: LinkRefused, Reason: err.Error()}
		}
		if d.tap != nil {
			select {
			case d.tap <- message:
			default:
			}
		}
		d.link.receive(message)
	}
}

// writeFrames sends the connection's bounded queue until shutdown.
func (d *LinkDriver) writeFrames(ctx context.Context, outgoing FrameSink) LinkEnd {
	for {
		select {
		case <-ctx.Done():
			return LinkEnd{Kind: LinkClosed, Reason: "runner disconnected"}
		case frame := <-d.link.outbound:
			if err := outgoing.Send(ctx, frame); err != nil {
				return LinkEnd{Kind: LinkClosed, Reason: "runner disconnected: " + err.Error()}
			}
		}
	}
}

// ping checks expiry before sending the next liveness probe.
func (l *Link) ping(ctx context.Context) (LinkEnd, bool) {
	l.mu.Lock()
	state := l.liveness
	if state == pingIdle {
		l.liveness = pingWaiting
	}
	l.mu.Unlock()
	if state == pingPaused {
		return LinkEnd{}, false
	}
	if state == pingWaiting {
		end := LinkEnd{Kind: LinkDisconnected, Reason: "liveness: ping unanswered"}
		return end, true
	}
	if err := l.send(ctx, &runnerproto.Ping{}); err != nil {
		end := LinkEnd{Kind: LinkClosed, Reason: "runner disconnected"}
		return end, true
	}
	return LinkEnd{}, false
}
