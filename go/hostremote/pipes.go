package hostremote

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"sync"
	"time"

	"github.com/wspl/demi/go/runnerproto"
)

const Arrival = 120 * time.Second

var (
	ErrPipeNotFound  = errors.New("no such pipe")
	ErrPipeConnected = errors.New("the end is already connected")
	ErrPipeSettled   = errors.New("the pipe is over")
)

// requestID names a runner request or pipe without exposing backend identities.
func requestID() string {
	var bytes [16]byte
	rand.Read(bytes[:])
	return hex.EncodeToString(bytes[:])
}

// Pipes owns rendezvous records. Byte copies happen outside its mutex; Close
// fails all records and joins every arrival watcher before returning.
type Pipes struct {
	mu      sync.Mutex
	slots   map[string]*Pipe
	arrival time.Duration
	closed  bool
	tasks   sync.WaitGroup
}

func NewPipes(arrival time.Duration) *Pipes {
	return &Pipes{slots: make(map[string]*Pipe), arrival: arrival}
}

type pipeEnd struct {
	device  string
	process bool
	arrived bool
	taken   bool
}
type Pipe struct {
	owner        *Pipes
	id           string
	source, sink pipeEnd
	deadline     time.Time
	changed      chan struct{}
	done         chan struct{}
	ctx          context.Context
	cancel       context.CancelFunc
	failure      error
	settled      bool
	chunks       chan []byte
}

func (p *Pipes) Mint(source, sink string) *Pipe {
	ctx, cancel := context.WithCancel(context.Background())
	pipe := &Pipe{owner: p, id: requestID(), source: pipeEnd{device: source}, sink: pipeEnd{device: sink}, deadline: time.Now().Add(p.arrival), changed: make(chan struct{}), done: make(chan struct{}), ctx: ctx, cancel: cancel, chunks: make(chan []byte, 1)}
	p.mu.Lock()
	if p.closed {
		pipe.settleLocked("backend shutting down")
	} else {
		p.slots[pipe.id] = pipe
		p.tasks.Add(1)
		go pipe.watch()
	}
	p.mu.Unlock()
	return pipe
}
func (p *Pipes) FromDevice(device string) *Pipe { return p.Mint(device, "") }
func (p *Pipes) ToDevice(device string) *Pipe   { return p.Mint("", device) }
func (p *Pipes) Pipe(id string) *Pipe {
	p.mu.Lock()
	defer p.mu.Unlock()
	return p.slots[id]
}
func (p *Pipe) ID() string { return p.id }
func (p *Pipe) WireRef() runnerproto.PipeRef {
	return runnerproto.PipeRef{ID: p.id, URL: "/api/pipes/" + p.id}
}
func (p *Pipe) signalLocked() {
	close(p.changed)
	p.changed = make(chan struct{})
}
func (p *Pipe) settleLocked(reason string) bool {
	if p.settled {
		return false
	}
	p.settled = true
	if reason != "" {
		p.failure = fmt.Errorf("pipe failed: %s", reason)
	}
	close(p.done)
	p.cancel()
	return true
}
func (p *Pipe) updateLocked() {
	if p.source.arrived && p.sink.arrived {
		p.deadline = time.Time{}
	} else if p.deadline.IsZero() {
		p.deadline = time.Now().Add(p.owner.arrival)
	}
	p.signalLocked()
}
func (p *Pipe) watch() {
	defer p.owner.tasks.Done()
	defer func() {
		p.owner.mu.Lock()
		delete(p.owner.slots, p.id)
		p.owner.mu.Unlock()
	}()
	timer := time.NewTimer(time.Hour)
	defer timer.Stop()
	for {
		p.owner.mu.Lock()
		deadline, changed := p.deadline, p.changed
		p.owner.mu.Unlock()
		timer.Stop()
		var expired <-chan time.Time
		if !deadline.IsZero() {
			timer.Reset(time.Until(deadline))
			expired = timer.C
		}
		select {
		case <-p.done:
			return
		case <-changed:
		case <-expired:
			p.owner.mu.Lock()
			if !p.deadline.IsZero() && !time.Now().Before(p.deadline) {
				p.settleLocked("an end never arrived")
			}
			p.owner.mu.Unlock()
		}
	}
}
func (p *Pipe) Fail(reason string) {
	p.owner.mu.Lock()
	defer p.owner.mu.Unlock()
	p.settleLocked(reason)
}
func (p *Pipe) Failure() error {
	p.owner.mu.Lock()
	defer p.owner.mu.Unlock()
	return p.failure
}
func (p *Pipe) Done(ctx context.Context) error {
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-p.done:
		return p.Failure()
	}
}
func (p *Pipes) Fail(id, reason string) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if pipe := p.slots[id]; pipe != nil {
		pipe.settleLocked(reason)
	}
}
func (p *Pipes) FailFromDevice(id, device, reason string) bool {
	p.mu.Lock()
	defer p.mu.Unlock()
	pipe := p.slots[id]
	return pipe != nil && (pipe.source.device == device || pipe.sink.device == device) && pipe.settleLocked(reason)
}
func (p *Pipes) DeviceGone(device string) {
	p.mu.Lock()
	defer p.mu.Unlock()
	for _, pipe := range p.slots {
		if pipe.source.device == device || pipe.sink.device == device {
			pipe.settleLocked("device " + device + " disconnected")
		}
	}
}
func (p *Pipes) Close() error {
	p.mu.Lock()
	p.closed = true
	for _, pipe := range p.slots {
		pipe.settleLocked("backend shutting down")
	}
	p.mu.Unlock()
	p.tasks.Wait()
	return nil
}
func (p *Pipe) nameEnd(source bool, device string) error {
	p.owner.mu.Lock()
	defer p.owner.mu.Unlock()
	if p.settled {
		return ErrPipeSettled
	}
	end, name := &p.sink, "sink"
	if source {
		end, name = &p.source, "source"
	}
	if end.device != "" || end.process {
		return fmt.Errorf("the pipe's %s is already fixed", name)
	}
	end.device = device
	end.arrived = false
	p.updateLocked()
	return nil
}
func (p *Pipe) SourceFrom(device string) error { return p.nameEnd(true, device) }
func (p *Pipe) SinkTo(device string) error     { return p.nameEnd(false, device) }
func (p *Pipe) HoldSource() error {
	p.owner.mu.Lock()
	defer p.owner.mu.Unlock()
	if p.settled {
		return ErrPipeSettled
	}
	if p.source.device != "" || p.source.process {
		return errors.New("the pipe's source is already fixed")
	}
	p.source.arrived = true
	p.updateLocked()
	return nil
}
func (p *Pipe) take(source bool, device string) error {
	p.owner.mu.Lock()
	defer p.owner.mu.Unlock()
	end, name := &p.sink, "sink"
	if source {
		end, name = &p.source, "source"
	}
	if device != "" {
		if p.settled || end.device != device {
			return ErrPipeNotFound
		}
		if end.taken {
			return ErrPipeConnected
		}
	} else {
		if p.settled {
			return ErrPipeSettled
		}
		if end.device != "" {
			return fmt.Errorf("the pipe's %s is already fixed", name)
		}
		if end.taken {
			return fmt.Errorf("the pipe's %s is already taken", name)
		}
		end.process = true
	}
	end.taken = true
	end.arrived = true
	p.updateLocked()
	return nil
}
func (p *Pipe) Reader() (*PipeReader, error) {
	if err := p.take(false, ""); err != nil {
		return nil, err
	}
	return &PipeReader{pipe: p}, nil
}
func (p *Pipe) Writer() (*PipeWriter, error) {
	if err := p.take(true, ""); err != nil {
		return nil, err
	}
	return &PipeWriter{pipe: p}, nil
}
func (p *Pipes) ClaimSource(id, device string) (*DeviceSource, error) {
	pipe := p.Pipe(id)
	if pipe == nil {
		return nil, ErrPipeNotFound
	}
	if err := pipe.take(true, device); err != nil {
		return nil, err
	}
	return &DeviceSource{writer: &PipeWriter{pipe: pipe}}, nil
}
func (p *Pipes) ClaimSink(id, device string) (*DeviceSink, error) {
	pipe := p.Pipe(id)
	if pipe == nil {
		return nil, ErrPipeNotFound
	}
	if err := pipe.take(false, device); err != nil {
		return nil, err
	}
	return &DeviceSink{PipeReader: &PipeReader{pipe: pipe, abandoned: "sink HTTP response disconnected before EOF"}}, nil
}

// PipeReader is a single-consumer stream. Close interrupts its pending read.
type PipeReader struct {
	pipe      *Pipe
	remainder []byte
	ended     bool
	abandoned string
}

func (r *PipeReader) Next(ctx context.Context) ([]byte, error) {
	if r.ended {
		return nil, io.EOF
	}
	if err := r.pipe.Failure(); err != nil {
		r.ended = true
		return nil, err
	}
	select {
	case <-ctx.Done():
		return nil, ctx.Err()
	case <-r.pipe.done:
		r.ended = true
		if err := r.pipe.Failure(); err != nil {
			return nil, err
		}
		return nil, io.EOF
	case chunk, ok := <-r.pipe.chunks:
		if err := r.pipe.Failure(); err != nil {
			r.ended = true
			return nil, err
		}
		if ok {
			return chunk, nil
		}
		r.ended = true
		r.pipe.owner.mu.Lock()
		r.pipe.settleLocked("")
		r.pipe.owner.mu.Unlock()
		return nil, io.EOF
	}
}
func (r *PipeReader) Read(dst []byte) (int, error) {
	if len(dst) == 0 {
		return 0, nil
	}
	if len(r.remainder) == 0 {
		chunk, err := r.Next(context.Background())
		if err != nil {
			return 0, err
		}
		r.remainder = chunk
	}
	n := copy(dst, r.remainder)
	r.remainder = r.remainder[n:]
	return n, nil
}
func (r *PipeReader) Close() error {
	r.pipe.owner.mu.Lock()
	defer r.pipe.owner.mu.Unlock()
	r.pipe.settleLocked(r.abandoned)
	return nil
}

// PipeWriter has one producer. Close sends clean EOF; Abort fails the pipe.
type PipeWriter struct {
	pipe   *Pipe
	closed bool
}

func (w *PipeWriter) WriteContext(ctx context.Context, bytes []byte) (int, error) {
	if w.closed {
		return 0, io.ErrClosedPipe
	}
	if len(bytes) == 0 {
		return 0, nil
	}
	if err := w.pipe.Failure(); err != nil {
		return 0, err
	}
	chunk := append([]byte(nil), bytes...)
	select {
	case <-ctx.Done():
		return 0, ctx.Err()
	case <-w.pipe.done:
		if err := w.pipe.Failure(); err != nil {
			return 0, err
		}
		return 0, errors.New("pipe failed: the sink stopped reading")
	case w.pipe.chunks <- chunk:
		return len(bytes), nil
	}
}
func (w *PipeWriter) Write(bytes []byte) (int, error) {
	return w.WriteContext(context.Background(), bytes)
}
func (w *PipeWriter) Close() error {
	if !w.closed {
		w.closed = true
		close(w.pipe.chunks)
	}
	return nil
}
func (w *PipeWriter) Abort(reason string) {
	w.pipe.Fail(reason)
	w.Close()
}

type DeviceSource struct{ writer *PipeWriter }

func (s *DeviceSource) Pump(ctx context.Context, body io.ReadCloser) error {
	// A failed rendezvous must interrupt an HTTP body that has stopped sending.
	pipeStopped := make(chan struct{})
	stopPipe := context.AfterFunc(s.writer.pipe.ctx, func() {
		defer close(pipeStopped)
		body.Close()
	})
	defer func() {
		if !stopPipe() {
			<-pipeStopped
		}
	}()
	callStopped := make(chan struct{})
	stopCall := context.AfterFunc(ctx, func() {
		defer close(callStopped)
		s.writer.pipe.Fail("source HTTP request disconnected")
		body.Close()
	})
	defer func() {
		if !stopCall() {
			<-callStopped
		}
	}()
	defer body.Close()
	_, err := io.Copy(s.writer, body)
	if err != nil {
		s.writer.Abort("source HTTP request disconnected: " + err.Error())
	} else {
		s.writer.Close()
	}
	return s.writer.pipe.Done(ctx)
}

type DeviceSink struct{ *PipeReader }

func (s *DeviceSink) SourceArrived(ctx context.Context) error {
	for {
		p := s.pipe
		p.owner.mu.Lock()
		arrived := p.source.taken
		changed := p.changed
		failure := p.failure
		p.owner.mu.Unlock()
		if failure != nil {
			return failure
		}
		if arrived {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-p.done:
			return p.Failure()
		case <-changed:
		}
	}
}
