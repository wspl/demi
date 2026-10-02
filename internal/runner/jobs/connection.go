//revive:disable:unused-parameter API checkpoint retains parameter names; bodies follow after merge.

package jobs

import (
	"context"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runnerwire"
)

// ConnectionHandle lets work reach the backend and the connection's owner.
// It does not own the backend socket. It is safe for concurrent use.
type ConnectionHandle struct {
	// Control is the owner's bounded queue of encoded replies and requests.
	// Only the connection owner closes it, after joining all senders.
	Control chan<- []byte
}

// NewConnectionHandle creates a handle and its bounded owner request channel.
// ctx lasts until the connection ends. The receiver is owned by the composition;
// it stops receiving on ctx cancellation, rather than waiting for channel closure.
func NewConnectionHandle(ctx context.Context, control chan<- []byte) (*ConnectionHandle, <-chan Request) {
	panic("not written: r-jobs")
}

// Done closes when the connection ends.
func (h *ConnectionHandle) Done() <-chan struct{} { panic("not written: r-jobs") }

// RegisterCall registers events until ended closes. cancel ends the call when
// it cannot keep up. The caller owns and joins call work and never closes events
// while the relay exists; channel closure is unnecessary for caller cleanup.
func (h *ConnectionHandle) RegisterCall(ctx context.Context, id string, events chan<- CallEvent, ended <-chan struct{}, cancel context.CancelFunc) error {
	panic("not written: r-jobs")
}

// Locate asks where sha256 is on behalf of live work, within the backend's
// 15-second answer window and the connection's lifetime.
func (h *ConnectionHandle) Locate(ctx context.Context, owner runnerwire.ArtifactOwner, sha256 string) (commandwire.ArtifactLocation, error) {
	panic("not written: r-jobs")
}

// RegisterContext makes execution live and transfers leases to the connection
// owner on success. On failure the caller releases leases. An abandoned request
// must not retain a registration or its leases.
func (h *ConnectionHandle) RegisterContext(ctx context.Context, execution *ExecutionContext, leases []*cmdpkgs.ServiceLease) error {
	panic("not written: r-jobs")
}

// Reserve requests conversation numbers with the same answer window as Locate.
func (h *ConnectionHandle) Reserve(ctx context.Context, conversation string, sequence commandwire.ServiceSequence, count uint32) (uint64, error) {
	panic("not written: r-jobs")
}

var _ cmdpkgs.NumberSource = (*ConnectionHandle)(nil)

// Request is work waiting for the composition's connection owner.
// The owner handles queued registrations before routing backend replies.
//
//sumtype:decl
type Request interface{ connectionRequest() }

// CallRequest registers a call's events until Ended closes. Cancel is invoked
// when the bounded event queue is full; the relay never waits for room.
type CallRequest struct {
	ID     string
	Events chan<- CallEvent
	Ended  <-chan struct{}
	Cancel context.CancelFunc
}

func (*CallRequest) connectionRequest() { panic("not written: r-jobs") }

// AskRequest asks the backend a question until the asker abandons its wait.
type AskRequest struct {
	Question  Question
	Abandoned context.Context
}

func (*AskRequest) connectionRequest() { panic("not written: r-jobs") }

// ContextRequest asks the connection owner to make an execution context live.
// The owner calls Register exactly once, rather than inserting directly, so
// registration and acknowledgement agree even if the requester is cancelled.
type ContextRequest struct{}

func (*ContextRequest) connectionRequest() { panic("not written: r-jobs") }

// Register inserts the context and acknowledges its caller without waiting.
// An abandoned request is acknowledged without insertion; leases remain the
// caller's on failure and transfer to table on success.
func (r *ContextRequest) Register(table *ContextTable) { panic("not written: r-jobs") }

// Question carries a backend question and its one-shot answer destination.
//
//sumtype:decl
type Question interface{ backendQuestion() }

// LocateQuestion asks where an artifact is on behalf of Owner.
// Only ConnectionHandle creates questions; consumers read their public fields.
type LocateQuestion struct {
	Owner  runnerwire.ArtifactOwner
	SHA256 string
}

func (*LocateQuestion) backendQuestion() { panic("not written: r-jobs") }

// Answer delivers the location or failure once without blocking; a departed
// asker needs no answer. Repeated answers are ignored.
func (q *LocateQuestion) Answer(location commandwire.ArtifactLocation, err error) {
	panic("not written: r-jobs")
}

// ReserveQuestion asks for numbers from a conversation's sequence.
type ReserveQuestion struct {
	Conversation string
	Sequence     commandwire.ServiceSequence
	Count        uint32
}

func (*ReserveQuestion) backendQuestion() { panic("not written: r-jobs") }

// Answer delivers the first number or failure without blocking, at most once.
func (q *ReserveQuestion) Answer(first uint64, err error) { panic("not written: r-jobs") }

// CallEvent is what the backend tells a callback invocation.
//
//sumtype:decl
type CallEvent interface{ callEvent() }

// CallPipes supplies the callback's independently flowing IO pipes.
type CallPipes struct {
	Stdin  *runnerwire.PipeRef
	Stdout runnerwire.PipeRef
}

func (*CallPipes) callEvent() { panic("not written: r-jobs") }

// CallStderr delivers a chunk of the callback's standard error.
type CallStderr struct{ Bytes []byte }

func (*CallStderr) callEvent() { panic("not written: r-jobs") }

// CallPull requests the next chunk of live stdin.
type CallPull struct{}

func (*CallPull) callEvent() { panic("not written: r-jobs") }

// CallExit supplies the callback's completion status.
type CallExit struct{ ExitCode uint8 }

func (*CallExit) callEvent() { panic("not written: r-jobs") }

// Relay routes callback events and backend answers. It owns cancellation watches
// and removes entries when their caller leaves. Methods are safe for concurrent use.
// Its owner must Close it before discarding the connection.
type Relay struct{}

// NewRelay creates routing state owned by the connection lifetime ctx.
func NewRelay(ctx context.Context) *Relay { panic("not written: r-jobs") }

// Call registers where a callback's events go until its Ended channel closes.
func (r *Relay) Call(request *CallRequest) { panic("not written: r-jobs") }

// Ask registers a question and returns the encoded backend request. Encoding
// failure leaves no registration; the owner answers Question with that error.
func (r *Relay) Ask(request *AskRequest) ([]byte, error) { panic("not written: r-jobs") }

// Route delivers messages answering calls or questions, returning false for
// unrelated messages. Delivery never waits for a callback to consume its events.
func (r *Relay) Route(message runnerwire.Inbound) bool { panic("not written: r-jobs") }

// Close cancels and joins cancellation watches and releases routing entries.
// If ctx ends first, a later Close can finish the join. It is idempotent.
func (r *Relay) Close(ctx context.Context) error { panic("not written: r-jobs") }
