package remotehost

//revive:disable:unused-parameter
// API checkpoint: keep parameter names for callers until the bodies are implemented.

import (
	"context"
	"time"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
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
	GrowVolume(context.Context, runnerwire.VolumeName, uint64) error
	// ReserveNumbers reserves count numbers of the conversation's sequence.
	ReserveNumbers(context.Context, string, commandwire.ServiceSequence, uint32) (uint64, error)
}

// JobOrigin records whose a job is when it starts.
type JobOrigin struct {
	Host    host.Key
	Context commandwire.CommandContext
	Caller  *host.JobCaller
}

// LinkOptions describes a runner connection. Nil Ping disables liveness checks.
type LinkOptions struct {
	Device   string
	Identity host.Identity
	Pipes    *Pipes
	Policy   LinkPolicy
	Ping     *time.Duration
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
	Kind   LinkEndKind
	Reason string
}

// Link is one runner connection. Pointer identity identifies the connection.
// Its driver owns and joins its workers before Serve returns.
type Link struct{ _ byte }

// WeakLink identifies a connection without retaining it.
type WeakLink struct{ _ byte }

// Is reports whether link is the connection this names.
func (w WeakLink) Is(link *Link) bool { panic("not written: b-remotehost") }

// NewLink creates a connection and its single driver.
func NewLink(options LinkOptions) (*Link, *LinkDriver) { panic("not written: b-remotehost") }

// Downgrade returns a non-retaining connection identity.
func (l *Link) Downgrade() WeakLink { panic("not written: b-remotehost") }

// Device returns the device whose pipe ends this connection uses.
func (l *Link) Device() string { panic("not written: b-remotehost") }

// Identity returns the account reported by the runner.
func (l *Link) Identity() host.Identity { panic("not written: b-remotehost") }

// Pipes returns the connection's pipe broker.
func (l *Link) Pipes() *Pipes { panic("not written: b-remotehost") }

// Installs returns an immutable snapshot of installation progress.
func (l *Link) Installs() []runnerwire.Install { panic("not written: b-remotehost") }

// WatchInstalls returns progress and a channel closed on the next change.
// Reload after notification; no subscription needs releasing.
func (l *Link) WatchInstalls() ([]runnerwire.Install, <-chan struct{}) {
	panic("not written: b-remotehost")
}

// IsClosed reports whether the connection ended.
func (l *Link) IsClosed() bool { panic("not written: b-remotehost") }

// Disconnect ends the connection for reason without waiting for its driver.
func (l *Link) Disconnect(reason string) { panic("not written: b-remotehost") }

// PauseLiveness suspends ping expiry while the device is paused.
func (l *Link) PauseLiveness() { panic("not written: b-remotehost") }

// ResumeLiveness resumes ping expiry after a device pause.
func (l *Link) ResumeLiveness() { panic("not written: b-remotehost") }

// RunningJobs counts jobs still running on this connection.
func (l *Link) RunningJobs() uint64 { panic("not written: b-remotehost") }

// Sync waits for the runner's synchronization reply.
func (l *Link) Sync(ctx context.Context) error { panic("not written: b-remotehost") }

// ReleaseConversation retires the conversation's resident services.
func (l *Link) ReleaseConversation(ctx context.Context, conversation string) error {
	panic("not written: b-remotehost")
}

// JobOutput holds one output view and its offset in the stream.
type JobOutput struct {
	Stream core.StreamKind
	Offset uint64
	Bytes  []byte
}

// JobEnd holds the job's terminal status, directory, output lengths and edits.
type JobEnd struct {
	Status         host.ProcessEnd
	CWD            *string
	Output         *runnerwire.OutputLengths
	Files          []runnerwire.JobFileChange
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
type LinkDriver struct{ _ byte }

// Tap copies decoded runner messages for wire audits; a full channel loses the copy.
// Set it before Serve. The caller owns the channel and closes it only after Serve.
func (d *LinkDriver) Tap(tap chan<- runnerwire.Outbound) { panic("not written: b-remotehost") }

// Serve routes incoming replies, sends queued frames and pings until the connection ends.
// Cancellation ends all in-flight work; Serve joins all workers before returning.
func (d *LinkDriver) Serve(ctx context.Context, incoming FrameSource, outgoing FrameSink) LinkEnd {
	panic("not written: b-remotehost")
}
