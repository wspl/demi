package jobs

import (
	"context"
	"errors"
	"sync"
	"time"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/runner/cmdpkgs"
	"github.com/wspl/demi/internal/runnerwire"
)

// ConnectionHandle lets work reach the backend and the connection's owner.
// It does not own the backend socket. It is safe for concurrent use.
type ConnectionHandle struct {
	// Control is the owner's bounded queue of encoded replies and requests.
	// Only the connection owner closes it, after joining all senders.
	Control  chan<- []byte
	lifetime context.Context
	cancel   context.CancelFunc
	requests chan Request
}

// NewConnectionHandle creates a handle and its bounded owner request channel.
// ctx lasts until the connection ends. The receiver is owned by the composition;
// it stops receiving on Done, rather than waiting for channel closure. A failed
// cleanup send also closes Done; the owner must then close the backend connection.
func NewConnectionHandle(ctx context.Context, control chan<- []byte) (*ConnectionHandle, <-chan Request) {
	ctx, cancel := context.WithCancel(ctx)
	requests := make(chan Request, 64)
	return &ConnectionHandle{Control: control, lifetime: ctx, cancel: cancel, requests: requests}, requests
}

// Done closes when the connection ends or cannot reliably send RPC cleanup.
func (h *ConnectionHandle) Done() <-chan struct{} {
	return h.lifetime.Done()
}

// RegisterCall registers events until ended closes. cancel ends the call when
// it cannot keep up. The caller owns and joins call work and never closes events
// while the relay exists; channel closure is unnecessary for caller cleanup.
func (h *ConnectionHandle) RegisterCall(
	ctx context.Context,
	id string,
	events chan<- CallEvent,
	ended <-chan struct{},
	cancel context.CancelFunc,
) error {
	return h.request(ctx, &CallRequest{ID: id, Events: events, Ended: ended, Cancel: cancel})
}

// Locate asks where sha256 is on behalf of live work, within the backend's
// 15-second answer window and the connection's lifetime.
func (h *ConnectionHandle) Locate(
	ctx context.Context,
	owner runnerwire.ArtifactOwner,
	sha256 string,
) (commandwire.ArtifactLocation, error) {
	wait, cancel := context.WithCancel(ctx)
	defer cancel()
	q := &LocateQuestion{Owner: owner, SHA256: sha256, ready: make(chan struct{})}
	if err := h.request(wait, &AskRequest{Question: q, Abandoned: wait}); err != nil {
		return nil, &cmdpkgs.RuntimeError{Kind: cmdpkgs.Cancelled, Cause: err}
	}
	if err := h.waitAnswer(wait, q.ready, "artifact location"); err != nil {
		return nil, err
	}
	if q.err != nil {
		return nil, &cmdpkgs.RuntimeError{Kind: cmdpkgs.LocationFailure, Detail: q.err.Error(), Cause: q.err}
	}
	return q.location, nil
}

// RegisterContext makes execution live and transfers leases to the connection
// owner on success. On failure the caller releases leases. An abandoned request
// must not retain a registration or its leases.
func (h *ConnectionHandle) RegisterContext(
	ctx context.Context,
	execution *ExecutionContext,
	leases []*cmdpkgs.ServiceLease,
) error {
	r := &ContextRequest{
		ctx:        ctx,
		connection: h.lifetime,
		execution:  execution,
		leases:     leases,
		ready:      make(chan struct{}),
	}
	if err := h.request(ctx, r); err != nil {
		return err
	}
	select {
	case <-r.ready:
	case <-ctx.Done():
		r.abandon(ctx.Err())
	case <-h.Done():
		r.abandon(errors.New("host connection closed"))
	}
	r.mu.Lock()
	err := r.err
	r.mu.Unlock()
	return err
}

// Reserve requests conversation numbers with the same answer window as Locate.
func (h *ConnectionHandle) Reserve(
	ctx context.Context,
	conversation string,
	sequence commandwire.ServiceSequence,
	count uint32,
) (uint64, error) {
	wait, cancel := context.WithCancel(ctx)
	defer cancel()
	q := &ReserveQuestion{Conversation: conversation, Sequence: sequence, Count: count, ready: make(chan struct{})}
	if err := h.request(wait, &AskRequest{Question: q, Abandoned: wait}); err != nil {
		return 0, &cmdpkgs.RuntimeError{Kind: cmdpkgs.Cancelled, Cause: err}
	}
	if err := h.waitAnswer(wait, q.ready, "conversation numbers"); err != nil {
		return 0, err
	}
	return q.first, q.err
}

// waitAnswer starts the backend's answer deadline only after the owner request is queued.
func (h *ConnectionHandle) waitAnswer(ctx context.Context, ready <-chan struct{}, what string) error {
	timer := time.NewTimer(15 * time.Second)
	defer timer.Stop()
	select {
	case <-ready:
		return nil
	case <-h.Done():
		return &cmdpkgs.RuntimeError{Kind: cmdpkgs.Cancelled, Cause: context.Canceled}
	case <-ctx.Done():
		return &cmdpkgs.RuntimeError{Kind: cmdpkgs.Cancelled, Cause: ctx.Err()}
	case <-timer.C:
		return &cmdpkgs.RuntimeError{Kind: cmdpkgs.Deadline, Detail: what, Cause: context.DeadlineExceeded}
	}
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
	// ID identifies the callback invocation.
	ID string
	// Events receives the callback events.
	Events chan<- CallEvent
	// Ended closes when the callback invocation ends.
	Ended <-chan struct{}
	// Cancel ends a call that cannot consume its events.
	Cancel context.CancelFunc
}

func (*CallRequest) connectionRequest() {}

// AskRequest asks the backend a question until the asker abandons its wait.
type AskRequest struct {
	// Question carries the backend question and its answer destination.
	Question Question
	// Abandoned ends the wait when the asker leaves.
	Abandoned context.Context
}

func (*AskRequest) connectionRequest() {}

// ContextRequest asks the connection owner to make an execution context live.
// The owner calls Register exactly once, rather than inserting directly, so
// registration and acknowledgement agree even if the requester is cancelled.
type ContextRequest struct {
	// mu makes abandonment and insertion one decision; no wait or IO runs under it.
	mu         sync.Mutex
	ctx        context.Context
	connection context.Context
	execution  *ExecutionContext
	leases     []*cmdpkgs.ServiceLease
	ready      chan struct{}
	finished   bool
	err        error
}

func (*ContextRequest) connectionRequest() {}

// Register inserts the context and acknowledges its caller without waiting.
// An abandoned request is acknowledged without insertion; leases remain the
// caller's on failure and transfer to table on success.
func (r *ContextRequest) Register(table *ContextTable) {
	r.mu.Lock()
	if r.finished {
		r.mu.Unlock()
		return
	}
	err := r.ctx.Err()
	if err == nil {
		err = r.connection.Err()
	}
	if err == nil {
		err = table.Insert(r.execution, r.leases)
	}
	r.err = err
	r.finished = true
	r.mu.Unlock()
	close(r.ready)
}

// Question carries a backend question and its one-shot answer destination.
//
//sumtype:decl
type Question interface{ backendQuestion() }

// LocateQuestion asks where an artifact is on behalf of Owner.
// Only ConnectionHandle creates questions; consumers read their public fields.
type LocateQuestion struct {
	// Owner identifies the authority requesting the artifact.
	Owner runnerwire.ArtifactOwner
	// SHA256 identifies the artifact bytes.
	SHA256   string
	once     sync.Once
	ready    chan struct{}
	location commandwire.ArtifactLocation
	err      error
}

func (*LocateQuestion) backendQuestion() {}

// Answer delivers the location or failure once without blocking; a departed
// asker needs no answer. Repeated answers are ignored.
func (q *LocateQuestion) Answer(location commandwire.ArtifactLocation, err error) {
	q.once.Do(func() {
		q.location = location
		q.err = err
		close(q.ready)
	})
}

// ReserveQuestion asks for numbers from a conversation's sequence.
type ReserveQuestion struct {
	// Conversation identifies the conversation requesting numbers.
	Conversation string
	// Sequence selects the conversation number sequence.
	Sequence commandwire.ServiceSequence
	// Count is the number of consecutive values requested.
	Count uint32
	once  sync.Once
	ready chan struct{}
	first uint64
	err   error
}

func (*ReserveQuestion) backendQuestion() {}

// Answer delivers the first number or failure without blocking, at most once.
func (q *ReserveQuestion) Answer(first uint64, err error) {
	q.once.Do(func() {
		q.first = first
		q.err = err
		close(q.ready)
	})
}

// CallEvent is what the backend tells a callback invocation.
//
//sumtype:decl
type CallEvent interface{ callEvent() }

// CallPipes supplies the callback's independently flowing IO pipes.
type CallPipes struct {
	// Stdin is the optional callback input pipe.
	Stdin *runnerwire.PipeRef
	// Stdout is the callback output pipe.
	Stdout runnerwire.PipeRef
}

func (*CallPipes) callEvent() {}

// CallStderr delivers a chunk of the callback's standard error.
type CallStderr struct {
	// Bytes contains the callback standard error chunk.
	Bytes []byte
}

func (*CallStderr) callEvent() {}

// CallPull requests the next chunk of live stdin.
type CallPull struct{}

func (*CallPull) callEvent() {}

// CallExit supplies the callback's completion status.
type CallExit struct {
	// ExitCode is the callback completion status.
	ExitCode uint8
}

func (*CallExit) callEvent() {}

// Relay routes callback events and backend answers. It owns cancellation watches
// and removes entries when their caller leaves. Methods are safe for concurrent use.
// Its owner must Close it before discarding the connection.
type Relay struct {
	// mu protects registrations and admission of cancellation watches.
	mu        sync.Mutex
	calls     map[string]*CallRequest
	asks      map[string]*relayedQuestion
	lifetime  context.Context
	cancel    context.CancelFunc
	watches   sync.WaitGroup
	closing   bool
	closeOnce sync.Once
	done      chan struct{}
}
type relayedQuestion struct {
	question Question
	done     chan struct{}
}

// NewRelay creates routing state owned by the connection lifetime ctx.
func NewRelay(ctx context.Context) *Relay {
	lifetime, cancel := context.WithCancel(ctx)
	return &Relay{
		calls:    make(map[string]*CallRequest),
		asks:     make(map[string]*relayedQuestion),
		lifetime: lifetime,
		cancel:   cancel,
		done:     make(chan struct{}),
	}
}

// Call registers where a callback's events go until its Ended channel closes.
func (r *Relay) Call(request *CallRequest) {
	r.mu.Lock()
	if r.closing {
		r.mu.Unlock()
		request.Cancel()
		return
	}
	r.calls[request.ID] = request
	r.watches.Add(1)
	r.mu.Unlock()
	go func() {
		defer r.watches.Done()
		select {
		case <-r.lifetime.Done():
			request.Cancel()
		case <-request.Ended:
		}
		r.mu.Lock()
		if r.calls[request.ID] == request {
			delete(r.calls, request.ID)
		}
		r.mu.Unlock()
	}()
}

// Ask registers a question and returns the encoded backend request. Encoding
// failure leaves no registration; the owner answers Question with that error.
func (r *Relay) Ask(request *AskRequest) ([]byte, error) {
	id := executionID()
	var message runnerwire.Outbound
	switch q := request.Question.(type) {
	case *LocateQuestion:
		target, err := commandwire.HostTarget()
		if err != nil {
			return nil, err
		}
		message = &runnerwire.ArtifactResolve{ID: id, Owner: q.Owner, SHA256: q.SHA256, Target: string(target)}
	case *ReserveQuestion:
		message = &runnerwire.NumbersReserve{
			ID:             id,
			ConversationID: q.Conversation,
			Sequence:       q.Sequence,
			Count:          q.Count,
		}
	}
	frame, err := runnerwire.Encode(message)
	if err != nil {
		return nil, err
	}
	entry := &relayedQuestion{question: request.Question, done: make(chan struct{})}
	r.mu.Lock()
	if r.closing {
		r.mu.Unlock()
		return nil, context.Canceled
	}
	r.asks[id] = entry
	r.watches.Add(1)
	r.mu.Unlock()
	go func() {
		defer r.watches.Done()
		select {
		case <-r.lifetime.Done():
		case <-request.Abandoned.Done():
		case <-entry.done:
		}
		r.mu.Lock()
		if r.asks[id] == entry {
			delete(r.asks, id)
		}
		r.mu.Unlock()
	}()
	return frame, nil
}

// Route delivers messages answering calls or questions, returning false for
// unrelated messages. Delivery never waits for a callback to consume its events.
func (r *Relay) Route(message runnerwire.Inbound) bool {
	var id string
	var event CallEvent
	// This router deliberately handles only callback and question replies.
	switch m := any(message).(type) {
	case *runnerwire.RPCPipes:
		id = m.CallID
		event = &CallPipes{Stdin: m.Stdin, Stdout: m.Stdout}
	case *runnerwire.RPCOutput:
		id = m.CallID
		event = &CallStderr{Bytes: append([]byte(nil), m.Bytes...)}
	case *runnerwire.RPCStdinPull:
		id = m.CallID
		event = &CallPull{}
	case *runnerwire.RPCExit:
		id = m.CallID
		event = &CallExit{ExitCode: m.ExitCode}
	case *runnerwire.ArtifactLocation:
		r.locateAnswer(m)
		return true
	case *runnerwire.NumbersReserved:
		if q, ok := r.answer(m.ID).(*ReserveQuestion); ok {
			var err error
			var first uint64
			if m.First != nil && m.Error == nil {
				first = *m.First
			} else if m.First == nil && m.Error != nil {
				err = errors.New(*m.Error)
			} else {
				err = errors.New("invalid numbers response")
			}
			q.Answer(first, err)
		}
		return true
	default:
		return false
	}
	r.mu.Lock()
	call := r.calls[id]
	r.mu.Unlock()
	if call != nil {
		select {
		case <-call.Ended:
		case call.Events <- event:
		default:
			call.Cancel()
		}
	}
	return true
}

// Close cancels and joins cancellation watches and releases routing entries.
// If ctx ends first, a later Close can finish the join. It is idempotent.
func (r *Relay) Close(ctx context.Context) error {
	r.closeOnce.Do(func() {
		r.mu.Lock()
		r.closing = true
		r.mu.Unlock()
		r.cancel()
		go func() {
			r.watches.Wait()
			close(r.done)
		}()
	})
	select {
	case <-r.done:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// request queues work while both its caller and connection remain alive.
func (h *ConnectionHandle) request(ctx context.Context, request Request) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-h.Done():
		return context.Canceled
	case h.requests <- request:
		return nil
	}
}

// abandon withdraws a context registration unless the connection already committed it.
func (r *ContextRequest) abandon(err error) {
	r.mu.Lock()
	if r.finished {
		r.mu.Unlock()
		return
	}
	r.finished = true
	r.err = err
	r.mu.Unlock()
	close(r.ready)
}

// answer removes one backend question and ends its cancellation watch.
func (r *Relay) answer(id string) Question {
	r.mu.Lock()
	entry := r.asks[id]
	delete(r.asks, id)
	r.mu.Unlock()
	if entry == nil {
		return nil
	}
	close(entry.done)
	return entry.question
}

func (r *Relay) locateAnswer(m *runnerwire.ArtifactLocation) {
	if q, ok := r.answer(m.ID).(*LocateQuestion); ok {
		var err error
		var location commandwire.ArtifactLocation
		if m.Location != nil && m.Error == nil {
			location = *m.Location
		} else if m.Location == nil && m.Error != nil {
			err = errors.New(*m.Error)
		} else {
			err = errors.New("invalid artifact location response")
		}
		q.Answer(location, err)
	}
}
