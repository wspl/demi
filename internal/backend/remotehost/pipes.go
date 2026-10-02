package remotehost

import (
	"context"
	"crypto/rand"
	"errors"
	"fmt"
	"io"
	"sync"
	"time"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// Arrival is the time allowed for both pipe ends to arrive.
const Arrival = 120 * time.Second

// PipeFailure is why a pipe failed; Message begins "pipe failed: ".
type PipeFailure struct{ Message string }

// Error describes the pipe failure.
func (e *PipeFailure) Error() string {
	return e.Message
}

// PipeRefusal identifies why a device cannot claim an end.
type PipeRefusal uint8

const (
	// PipeNotFound means no such pipe, an ended pipe, or another device's end.
	PipeNotFound PipeRefusal = iota
	// PipeAlreadyConnected means the end has already been claimed.
	PipeAlreadyConnected
)

// Error describes why the device cannot claim the end.
func (e PipeRefusal) Error() string {
	switch e {
	case PipeNotFound:
		return "no such pipe"
	case PipeAlreadyConnected:
		return "the end is already connected"
	}
	return "unknown pipe refusal"
}

// PipeErrorKind identifies why the backend cannot take an end.
type PipeErrorKind uint8

const (
	// PipeSettled means the pipe is over.
	PipeSettled PipeErrorKind = iota
	// PipeAlreadyFixed means the end's assignment is fixed.
	PipeAlreadyFixed
	// PipeTaken means the end has already been taken.
	PipeTaken
)

// PipeError describes an unavailable backend end (source or sink).
type PipeError struct {
	Kind PipeErrorKind
	End  string
}

// Error describes why the backend cannot take the end.
func (e *PipeError) Error() string {
	switch e.Kind {
	case PipeSettled:
		return "the pipe is over"
	case PipeAlreadyFixed:
		return fmt.Sprintf("the pipe's %s is already fixed", e.End)
	case PipeTaken:
		return fmt.Sprintf("the pipe's %s is already taken", e.End)
	}
	return "unknown pipe error"
}

// Pipes owns pipe rendezvous records and their expiry workers.
// Its owner must Close it; pipe ends may be handed to other goroutines.
type Pipes struct {
	mu      sync.Mutex // Protects records and worker admission.
	slots   map[string]*Pipe
	arrival time.Duration
	closed  bool
	workers sync.WaitGroup
}

// NewPipes creates a broker with the supplied arrival timeout.
func NewPipes(arrival time.Duration) *Pipes {
	return &Pipes{slots: make(map[string]*Pipe), arrival: arrival}
}

// Mint creates a pipe; nil endpoints are initially unassigned.
func (p *Pipes) Mint(source, sink *string) *Pipe {
	pipe := &Pipe{id: rand.Text(), broker: p, changed: make(chan struct{}), settled: make(chan struct{}), deadline: time.Now().Add(p.arrival)}
	if source != nil {
		pipe.source = pipeEnd{device: *source, kind: deviceEnd}
	}
	if sink != nil {
		pipe.sink = pipeEnd{device: *sink, kind: deviceEnd}
	}
	p.mu.Lock()
	if p.closed {
		p.mu.Unlock()
		pipe.Fail("backend shutting down")
		return pipe
	}
	p.slots[pipe.id] = pipe
	p.workers.Add(1)
	p.mu.Unlock()
	go pipe.watchArrival()
	return pipe
}

// FromDevice creates a pipe sourced by device.
func (p *Pipes) FromDevice(device string) *Pipe {
	return p.Mint(&device, nil)
}

// ToDevice creates a pipe drained by device.
func (p *Pipes) ToDevice(device string) *Pipe {
	return p.Mint(nil, &device)
}

// Pipe looks up a live pipe by ID.
func (p *Pipes) Pipe(id string) (*Pipe, bool) {
	p.mu.Lock()
	defer p.mu.Unlock()
	pipe, ok := p.slots[id]
	return pipe, ok
}

// ClaimSource takes the source end assigned to device.
func (p *Pipes) ClaimSource(id, device string) (*DeviceSource, error) {
	pipe, ok := p.Pipe(id)
	if !ok {
		return nil, PipeNotFound
	}
	pipe.mu.Lock()
	defer pipe.publish()
	if pipe.outcome != pipeOpen || pipe.source != (pipeEnd{device: device, kind: deviceEnd}) {
		return nil, PipeNotFound
	}
	if pipe.sourceTaken {
		return nil, PipeAlreadyConnected
	}
	pipe.sourceTaken = true
	pipe.sourceArrived = true
	pipe.sending = true
	pipe.updateDeadline()
	return &DeviceSource{writer: &PipeWriter{pipe: pipe}}, nil
}

// ClaimSink takes the sink end assigned to device.
func (p *Pipes) ClaimSink(id, device string) (*DeviceSink, error) {
	pipe, ok := p.Pipe(id)
	if !ok {
		return nil, PipeNotFound
	}
	pipe.mu.Lock()
	defer pipe.publish()
	if pipe.outcome != pipeOpen || pipe.sink != (pipeEnd{device: device, kind: deviceEnd}) {
		return nil, PipeNotFound
	}
	if pipe.sinkTaken {
		return nil, PipeAlreadyConnected
	}
	pipe.sinkTaken = true
	pipe.sinkArrived = true
	pipe.updateDeadline()
	return &DeviceSink{reader: &PipeReader{pipe: pipe, abandoned: "sink HTTP request disconnected"}}, nil
}

// Fail fails the named pipe for both ends.
func (p *Pipes) Fail(id, reason string) {
	if pipe, ok := p.Pipe(id); ok {
		pipe.Fail(reason)
	}
}

// FailFromDevice fails a pipe only if device owns one of its ends.
func (p *Pipes) FailFromDevice(id, device, reason string) bool {
	pipe, ok := p.Pipe(id)
	if !ok {
		return false
	}
	pipe.mu.Lock()
	party := pipeEnd{kind: deviceEnd, device: device}
	if pipe.outcome != pipeOpen || (pipe.source != party && pipe.sink != party) {
		pipe.mu.Unlock()
		return false
	}
	pipe.finishLocked(&PipeFailure{Message: "pipe failed: " + reason})
	return true
}

// DeviceGone fails all ends belonging to device.
func (p *Pipes) DeviceGone(device string) {
	p.mu.Lock()
	slots := make([]*Pipe, 0, len(p.slots))
	for _, pipe := range p.slots {
		slots = append(slots, pipe)
	}
	p.mu.Unlock()
	for _, pipe := range slots {
		p.FailFromDevice(pipe.id, device, "device "+device+" disconnected")
	}
}

// Close fails remaining pipes and joins expiry workers. It is idempotent.
func (p *Pipes) Close(ctx context.Context) error {
	p.mu.Lock()
	p.closed = true
	slots := make([]*Pipe, 0, len(p.slots))
	for _, pipe := range p.slots {
		slots = append(slots, pipe)
	}
	p.mu.Unlock()
	for _, pipe := range slots {
		pipe.Fail("backend shutting down")
	}
	// Shutdown always joins the already-interrupted expiry workers, including with a canceled context.
	p.workers.Wait()
	return ctx.Err()
}

// Pipe is a handle to a broker record, independent of ownership of its ends.
type Pipe struct {
	id                         string
	broker                     *Pipes
	mu                         sync.Mutex // Protects endpoints, the single queued chunk, deadline and outcome.
	changed                    chan struct{}
	settled                    chan struct{}
	source, sink               pipeEnd
	sourceArrived, sinkArrived bool
	sending                    bool
	sourceTaken, sinkTaken     bool
	deadline                   time.Time
	outcome                    pipeOutcome
	failure                    *PipeFailure
	queued                     []byte
	eof                        bool
}

// ID returns the pipe's identifier.
func (p *Pipe) ID() string {
	return p.id
}

// WireRef returns the pipe reference sent to the runner.
func (p *Pipe) WireRef() runnerwire.PipeRef {
	return runnerwire.PipeRef{ID: p.id, URL: "/api/pipes/" + p.id}
}

// Reader takes the backend sink. The caller must close it or consume it to EOF.
func (p *Pipe) Reader() (*PipeReader, error) {
	p.mu.Lock()
	defer p.publish()
	if p.outcome != pipeOpen {
		return nil, &PipeError{Kind: PipeSettled}
	}
	if p.sink.kind == deviceEnd {
		return nil, &PipeError{Kind: PipeAlreadyFixed, End: "sink"}
	}
	if p.sinkTaken {
		return nil, &PipeError{Kind: PipeTaken, End: "sink"}
	}
	p.sink = pipeEnd{kind: processPipeEnd}
	p.sinkTaken = true
	p.sinkArrived = true
	p.updateDeadline()
	return &PipeReader{pipe: p}, nil
}

// Writer takes the backend source. The caller must End or Fail it.
func (p *Pipe) Writer() (*PipeWriter, error) {
	p.mu.Lock()
	defer p.publish()
	if p.outcome != pipeOpen {
		return nil, &PipeError{Kind: PipeSettled}
	}
	if p.source.kind == deviceEnd {
		return nil, &PipeError{Kind: PipeAlreadyFixed, End: "source"}
	}
	if p.sourceTaken {
		return nil, &PipeError{Kind: PipeTaken, End: "source"}
	}
	p.source = pipeEnd{kind: processPipeEnd}
	p.sourceTaken = true
	p.sourceArrived = true
	p.sending = true
	p.updateDeadline()
	return &PipeWriter{pipe: p}, nil
}

// HoldSource reserves an unassigned source for later attachment.
func (p *Pipe) HoldSource() error {
	p.mu.Lock()
	defer p.publish()
	if p.outcome != pipeOpen {
		return &PipeError{Kind: PipeSettled}
	}
	if p.source.kind != openEnd {
		return &PipeError{Kind: PipeAlreadyFixed, End: "source"}
	}
	p.sourceArrived = true
	p.updateDeadline()
	return nil
}

// SinkTo assigns the sink to device.
func (p *Pipe) SinkTo(device string) error {
	return p.assignDevice(false, device)
}

// SourceFrom assigns the source to device.
func (p *Pipe) SourceFrom(device string) error {
	return p.assignDevice(true, device)
}

// Fail fails both ends with reason.
func (p *Pipe) Fail(reason string) {
	p.mu.Lock()
	p.finishLocked(&PipeFailure{Message: "pipe failed: " + reason})
}

// Done waits for successful draining or failure.
func (p *Pipe) Done(ctx context.Context) error {
	select {
	case <-p.settled:
		return p.failureError()
	default:
	}
	select {
	case <-p.settled:
		return p.failureError()
	case <-ctx.Done():
		return ctx.Err()
	}
}

// Failure returns the current failure, or nil while open or successfully drained.
func (p *Pipe) Failure() *PipeFailure {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.failure == nil {
		return nil
	}
	return new(*p.failure)
}

// Failed waits for failure; successful draining does not end this wait.
func (p *Pipe) Failed(ctx context.Context) (*PipeFailure, error) {
	if err := p.Done(ctx); err != nil {
		if failure := p.Failure(); failure != nil {
			return failure, nil
		}
		return nil, err
	}
	<-ctx.Done()
	return nil, ctx.Err()
}

// PipeReader is the backend's owned reading end. Do not read it concurrently.
type PipeReader struct {
	pipe      *Pipe
	pending   []byte
	ended     bool
	abandoned string
}

// Next returns the next byte chunk, or io.EOF once drained.
func (r *PipeReader) Next(ctx context.Context) ([]byte, error) {
	if r.ended {
		return nil, io.EOF
	}
	p := r.pipe
	for {
		p.mu.Lock()
		if p.failure != nil {
			err := p.failure
			p.mu.Unlock()
			r.ended = true
			return nil, err
		}
		if len(p.queued) > 0 {
			chunk := p.queued
			p.queued = nil
			p.publish()
			return chunk, nil
		}
		if p.eof || p.outcome == pipeDrained {
			p.finishLocked(nil)
			r.ended = true
			return nil, io.EOF
		}
		changed := p.changed
		p.mu.Unlock()
		select {
		case <-changed:
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
}

// Read reads bytes with cancellation and implements host.ByteStream.
func (r *PipeReader) Read(ctx context.Context, bytes []byte) (int, error) {
	if len(bytes) == 0 {
		return 0, nil
	}
	if len(r.pending) == 0 {
		chunk, err := r.Next(ctx)
		if err != nil {
			return 0, err
		}
		r.pending = chunk
	}
	n := copy(bytes, r.pending)
	r.pending = r.pending[n:]
	return n, nil
}

// Fail abandons the reader and fails both ends.
func (r *PipeReader) Fail(reason string) {
	r.ended = true
	r.pending = nil
	r.pipe.Fail(reason)
}

// Close releases the backend reader and marks an early read as drained. It is idempotent.
func (r *PipeReader) Close(_ context.Context) error {
	if !r.ended {
		r.ended = true
		r.pending = nil
		if r.abandoned != "" {
			r.pipe.Fail(r.abandoned)
		} else {
			r.pipe.mu.Lock()
			r.pipe.finishLocked(nil)
		}
	}
	return nil
}

// PipeWriter is the backend's owned writing end. Do not write it concurrently.
// End or Fail releases it; losing a Go reference does not close a pipe.
type PipeWriter struct {
	pipe  *Pipe
	ended bool
}

// Write waits until the reader has room for bytes.
func (w *PipeWriter) Write(ctx context.Context, bytes []byte) error {
	if len(bytes) == 0 {
		return nil
	}
	p := w.pipe
	for {
		p.mu.Lock()
		if p.outcome != pipeOpen || w.ended {
			err := p.failure
			p.mu.Unlock()
			if err != nil {
				return err
			}
			return &PipeFailure{Message: "pipe failed: the sink stopped reading"}
		}
		if len(p.queued) == 0 {
			p.queued = append([]byte(nil), bytes...)
			p.publish()
			return nil
		}
		changed := p.changed
		p.mu.Unlock()
		select {
		case <-changed:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
}

// End ends input cleanly and releases the writer.
func (w *PipeWriter) End() {
	if w.ended {
		return
	}
	w.ended = true
	w.pipe.mu.Lock()
	w.pipe.eof = true
	w.pipe.publish()
}

// Fail fails both ends and releases the writer.
func (w *PipeWriter) Fail(reason string) {
	if !w.ended {
		w.ended = true
		w.pipe.Fail(reason)
	}
}

// DeviceSource is a claimed device source; Pump or Fail must release it.
type DeviceSource struct{ writer *PipeWriter }

// Pump moves the body into the pipe, closes the body and releases the source on every path.
func (s *DeviceSource) Pump(ctx context.Context, body host.ByteStream) (result error) {
	p := s.writer.pipe
	readCtx, cancel := context.WithCancel(ctx)
	watched := make(chan struct{})
	go func() {
		defer close(watched)
		select {
		case <-p.settled:
			cancel()
		case <-readCtx.Done():
		}
	}()
	defer func() {
		cancel()
		<-watched
	}()
	defer s.writer.Fail("source HTTP request disconnected")
	defer func() {
		if err := body.Close(context.WithoutCancel(ctx)); err != nil {
			p.Fail("source HTTP request disconnected: " + err.Error())
			result = errors.Join(result, err)
		}
	}()
	buffer := make([]byte, 64*1024)
	for {
		n, err := body.Read(readCtx, buffer)
		if n > 0 {
			if writeErr := s.writer.Write(readCtx, buffer[:n]); writeErr != nil {
				select {
				case <-p.settled:
					return p.failureError()
				default:
				}
				p.Fail("source HTTP request disconnected")
				return p.Done(context.WithoutCancel(ctx))
			}
		}
		if err != nil {
			select {
			case <-p.settled:
				return p.failureError()
			default:
			}
			if errors.Is(err, io.EOF) {
				s.writer.End()
				if err := p.Done(ctx); err != nil {
					if ctx.Err() != nil {
						p.Fail("source HTTP request disconnected")
					}
					return p.Done(context.WithoutCancel(ctx))
				}
				return nil
			}
			if ctx.Err() != nil {
				p.Fail("source HTTP request disconnected")
			} else {
				p.Fail("source HTTP request disconnected: " + err.Error())
			}
			return p.Done(context.WithoutCancel(ctx))
		}
	}
}

// Fail releases an abandoned claim and fails the pipe.
func (s *DeviceSource) Fail(reason string) {
	s.writer.Fail(reason)
}

// DeviceSink is a claimed device sink; its owner closes it or drains it to EOF.
type DeviceSink struct{ reader *PipeReader }

// SourceArrived waits until the source is present or the pipe fails.
func (s *DeviceSink) SourceArrived(ctx context.Context) error {
	p := s.reader.pipe
	for {
		p.mu.Lock()
		if p.failure != nil {
			err := p.failure
			p.mu.Unlock()
			return err
		}
		if p.sending {
			p.mu.Unlock()
			return nil
		}
		changed := p.changed
		p.mu.Unlock()
		select {
		case <-changed:
		case <-ctx.Done():
			return ctx.Err()
		}
	}
}

// Next returns the next byte chunk, or io.EOF once drained.
func (s *DeviceSink) Next(ctx context.Context) ([]byte, error) {
	s.reader.abandoned = "sink HTTP response disconnected before EOF"
	return s.reader.Next(ctx)
}

// Read reads bytes with cancellation.
func (s *DeviceSink) Read(ctx context.Context, bytes []byte) (int, error) {
	s.reader.abandoned = "sink HTTP response disconnected before EOF"
	return s.reader.Read(ctx, bytes)
}

// Close releases the sink, failing an incompletely drained pipe. It is idempotent.
func (s *DeviceSink) Close(ctx context.Context) error {
	return s.reader.Close(ctx)
}

var _ host.ByteStream = (*PipeReader)(nil)
var _ host.ByteStream = (*DeviceSink)(nil)

// pipeEnd identifies which participant may take one end of a runner pipe.
type pipeEnd struct {
	kind   pipeEndKind
	device string
}
type pipeEndKind uint8

const (
	openEnd pipeEndKind = iota
	processPipeEnd
	deviceEnd
)

type pipeOutcome uint8

const (
	pipeOpen pipeOutcome = iota
	pipeDrained
	pipeFailed
)

// publish announces a pipe mutation after releasing its mutex.
func (p *Pipe) publish() {
	previous := p.changed
	p.changed = make(chan struct{})
	p.mu.Unlock()
	close(previous)
}

// finishLocked settles a pipe once, releasing its lock before notifications.
func (p *Pipe) finishLocked(failure *PipeFailure) {
	if p.outcome != pipeOpen {
		p.mu.Unlock()
		return
	}
	if failure == nil {
		p.outcome = pipeDrained
	} else {
		p.outcome = pipeFailed
		p.failure = failure
	}
	p.publish()
	close(p.settled)
}

// failureError avoids placing a nil typed pipe failure in an error interface.
func (p *Pipe) failureError() error {
	if failure := p.Failure(); failure != nil {
		return failure
	}
	return nil
}

// updateDeadline restarts arrival only when a previously complete pair loses an end.
func (p *Pipe) updateDeadline() {
	if p.sourceArrived && p.sinkArrived {
		p.deadline = time.Time{}
	} else if p.deadline.IsZero() {
		p.deadline = time.Now().Add(p.broker.arrival)
	}
}

// assignDevice names a still-unassigned pipe end and restarts arrival if needed.
func (p *Pipe) assignDevice(source bool, device string) error {
	p.mu.Lock()
	defer p.publish()
	if p.outcome != pipeOpen {
		return &PipeError{Kind: PipeSettled}
	}
	end := &p.sink
	name := "sink"
	if source {
		end = &p.source
		name = "source"
	}
	if end.kind != openEnd {
		return &PipeError{Kind: PipeAlreadyFixed, End: name}
	}
	*end = pipeEnd{kind: deviceEnd, device: device}
	if source {
		p.sourceArrived = false
	} else {
		p.sinkArrived = false
	}
	p.updateDeadline()
	return nil
}

// watchArrival owns a pipe's timer and removes the broker record after settlement.
func (p *Pipe) watchArrival() {
	defer p.broker.workers.Done()
	defer func() {
		p.broker.mu.Lock()
		delete(p.broker.slots, p.id)
		p.broker.mu.Unlock()
	}()
	timer := time.NewTimer(time.Hour)
	defer timer.Stop()
	for {
		p.mu.Lock()
		if p.outcome != pipeOpen {
			p.mu.Unlock()
			return
		}
		deadline := p.deadline
		changed := p.changed
		p.mu.Unlock()
		timer.Stop()
		var expiry <-chan time.Time
		if !deadline.IsZero() {
			timer.Reset(time.Until(deadline))
			expiry = timer.C
		}
		select {
		case <-p.settled:
			return
		case <-changed:
		case <-expiry:
			p.mu.Lock()
			if !p.deadline.IsZero() && !time.Now().Before(p.deadline) {
				p.finishLocked(&PipeFailure{Message: "pipe failed: an end never arrived"})
			} else {
				p.mu.Unlock()
			}
		}
	}
}
