package commandservice

import (
	"context"
	"errors"
	"fmt"
	"io"
	"math"
	"net"
	"net/http"
	"slices"
	"sync"
	"sync/atomic"
	"time"
)

const cancellationGrace = 5 * time.Second
const phaseTimeout = 10 * time.Second

const (
	serviceRunning uint32 = iota
	serviceDraining
	serviceStopped
)

// Handler owns invocation work. Invoke must stop when Call.Context ends.
// A handler that ignores cancellation may outlive Serve only when Serve returns
// ErrCancellationDeadline; the process owner must then exit to retire that work.
type Handler interface {
	Operations() []string
	Invoke(*Call) (Completion, error)
}

// ConversationHandler optionally owns conversation status and resource release.
type ConversationHandler interface {
	Conversation(*ConversationCall) (Completion, error)
}

// Closer optionally releases service resources after invocation handlers stop.
// Close must honor its deadline. A noncooperative Close may outlive Serve only
// after ErrCancellationDeadline, requiring the process owner to exit.
type Closer interface{ Close(context.Context) error }

// Call contains one invocation's metadata, bounded IO, and numbers source.
type Call struct {
	Invocation Invocation
	Stdin      *Input
	Stdout     io.Writer
	Stderr     io.Writer
	Numbers    *Numbers
	ctx        context.Context
}

// Context ends on cancellation and when the invocation finishes.
func (c *Call) Context() context.Context { return c.ctx }

// ConversationCall contains trusted lifecycle metadata and its output writer.
type ConversationCall struct {
	Request ConversationRequest
	Stdout  io.Writer
	ctx     context.Context
}

// Context ends on cancellation and when the lifecycle call finishes.
func (c *ConversationCall) Context() context.Context { return c.ctx }

// Input supplies one bounded chunk per Next call, preserving input boundaries.
type Input struct {
	reader io.Reader
	output *recordOutput
	ended  bool
}

// Next requests one chunk, or returns io.EOF once the caller ends its input.
func (i *Input) Next() ([]byte, error) {
	if i.ended {
		return nil, io.EOF
	}
	if err := i.output.send(Record{Kind: InputPull}); err != nil {
		return nil, err
	}
	data, err := readFrame(i.reader, MaxRecordBytes)
	if err == io.EOF {
		i.ended = true
	}
	return data, err
}

type recordOutput struct {
	ctx     context.Context
	records chan Record
}

func (o *recordOutput) send(record Record) error {
	if o.ctx.Err() != nil {
		return ErrCancelled
	}
	select {
	case <-o.ctx.Done():
		return ErrCancelled
	case o.records <- record:
		return nil
	}
}

type recordWriter struct {
	output *recordOutput
	kind   RecordKind
}

func (w recordWriter) Write(data []byte) (int, error) {
	written := 0
	for len(data) > 0 {
		count := min(len(data), MaxRecordBytes)
		record := Record{Kind: w.kind, Data: slices.Clone(data[:count])}
		if err := w.output.send(record); err != nil {
			return written, err
		}
		data = data[count:]
		written += count
	}
	return written, nil
}

func protocols() *http.Protocols {
	p := new(http.Protocols)
	p.SetUnencryptedHTTP2(true)
	return p
}

func http2Config() *http.HTTP2Config {
	return &http.HTTP2Config{
		MaxConcurrentStreams:          math.MaxInt32,
		MaxReceiveBufferPerStream:     MaxRecordBytes,
		MaxReceiveBufferPerConnection: math.MaxInt32,
	}
}

type service struct {
	handler       Handler
	info          ServiceInfo
	server        *http.Server
	numbers       *Numbers
	ctx           context.Context
	cancel        context.CancelFunc
	state         uint32
	fault         chan error
	shutdown      chan struct{}
	admission     sync.Mutex
	requests      map[chan struct{}]struct{}
	numbersOpened bool
}

func (s *service) fail(err error) {
	select {
	case s.fault <- err:
	default:
	}
	s.cancel()
	// Closing is best-effort teardown; the originating fault is retained.
	_ = s.server.Close()
}

func (s *service) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	done := make(chan struct{})
	s.admission.Lock()
	if s.state == serviceStopped {
		s.admission.Unlock()
		w.WriteHeader(http.StatusServiceUnavailable)
		return
	}
	shutdown := r.Method == http.MethodPost && r.URL.Path == ShutdownPath
	if s.state != serviceRunning && !shutdown {
		s.admission.Unlock()
		w.WriteHeader(http.StatusServiceUnavailable)
		return
	}
	firstShutdown := shutdown && s.state == serviceRunning
	if firstShutdown {
		s.state = serviceDraining
	}
	s.requests[done] = struct{}{}
	s.admission.Unlock()
	defer func() {
		s.admission.Lock()
		delete(s.requests, done)
		close(done)
		s.admission.Unlock()
	}()
	if shutdown {
		s.numbers.stop()
		w.WriteHeader(http.StatusOK)
		if err := http.NewResponseController(w).Flush(); err != nil {
			s.fail(err)
			return
		}
		if firstShutdown {
			close(s.shutdown)
		}
		return
	}
	switch {
	case r.Method == http.MethodGet && r.URL.Path == InfoPath:
		data, err := Encode(s.info)
		if err != nil {
			s.fail(err)
			return
		}
		if _, err = w.Write(data); err != nil {
			return
		}
	case r.Method == http.MethodPost && (r.URL.Path == InvokePath || r.URL.Path == ConversationPath || r.URL.Path == NumbersPath):
		s.invoke(w, r)
	default:
		w.WriteHeader(http.StatusNotFound)
	}
}

type handlerResult struct {
	completion Completion
	err        error
	panicked   bool
}

// runHandler retains the handler outcome independently of its output queue.
func runHandler(run func() (Completion, error), output *recordOutput, result chan<- handlerResult) {
	outcome := handlerResult{}
	defer func() {
		if cause := recover(); cause != nil {
			outcome.err = fmt.Errorf("command handler failed: %v", cause)
			outcome.panicked = true
		}
		result <- outcome
		close(output.records)
	}()
	outcome.completion, outcome.err = run()
}

func releaseFault(result handlerResult) error {
	if result.err != nil {
		if errors.Is(result.err, ErrCancelled) && !result.panicked {
			return nil
		}
		return fmt.Errorf("%w: %v", ErrConversationCleanup, result.err)
	}
	if result.completion.ExitCode != 0 {
		detail := fmt.Sprintf("release exited with status %d", result.completion.ExitCode)
		if commandErr := result.completion.Error; commandErr != nil {
			detail += fmt.Sprintf(": %s: %s", commandErr.Code, commandErr.Message)
		}
		return fmt.Errorf("%w: %s", ErrConversationCleanup, detail)
	}
	return nil
}

func (s *service) invoke(w http.ResponseWriter, r *http.Request) {
	controller := http.NewResponseController(w)
	if err := controller.SetReadDeadline(time.Now().Add(phaseTimeout)); err != nil {
		w.WriteHeader(http.StatusBadRequest)
		return
	}
	metadata, err := readFrame(r.Body, MaxMetadataBytes)
	if resetErr := controller.SetReadDeadline(time.Time{}); resetErr != nil {
		w.WriteHeader(http.StatusBadRequest)
		return
	}
	if err != nil {
		w.WriteHeader(http.StatusBadRequest)
		return
	}
	ctx, cancel := context.WithCancel(r.Context())
	stop := context.AfterFunc(s.ctx, cancel)
	defer stop()
	defer cancel()
	output := &recordOutput{ctx: ctx, records: make(chan Record, 4)}
	stdout := recordWriter{output: output, kind: Stdout}
	input := &Input{reader: r.Body, output: output}
	var run func() (Completion, error)
	release := false
	switch r.URL.Path {
	case InvokePath:
		invocation, err := Decode[Invocation](metadata)
		if err != nil {
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		if !slices.Contains(s.info.Operations, invocation.Operation) {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		call := &Call{
			Invocation: invocation,
			Stdin:      input,
			Stdout:     stdout,
			Stderr:     recordWriter{output: output, kind: Stderr},
			Numbers:    s.numbers,
			ctx:        ctx,
		}
		run = func() (Completion, error) { return s.handler.Invoke(call) }
	case ConversationPath:
		request, err := Decode[ConversationRequest](metadata)
		if err != nil {
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		release = request.Operation == "release"
		run = func() (Completion, error) {
			if handler, ok := s.handler.(ConversationHandler); ok {
				return handler.Conversation(&ConversationCall{Request: request, Stdout: stdout, ctx: ctx})
			}
			body := []byte(`{}`)
			if request.Operation == "status" {
				body = []byte(`{"conversations":[]}`)
			}
			_, err := stdout.Write(body)
			return Completion{}, err
		}
	case NumbersPath:
		if _, err := Decode[NumbersOpen](metadata); err != nil {
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		s.admission.Lock()
		if s.numbersOpened {
			s.admission.Unlock()
			w.WriteHeader(http.StatusConflict)
			return
		}
		s.numbersOpened = true
		s.admission.Unlock()
		defer s.numbers.stop()
		run = func() (Completion, error) { return s.numbers.relay(ctx, input, r.Body) }
	}
	w.WriteHeader(http.StatusOK)
	if err := controller.Flush(); err != nil {
		return
	}
	result := make(chan handlerResult, 1)
	go runHandler(run, output, result)
	joined := false
	defer func() {
		cancel()
		// Closing a request body interrupts input; any read failure is already a stream failure.
		_ = r.Body.Close()
		if joined {
			return
		}
		timer := time.NewTimer(cancellationGrace)
		defer timer.Stop()
		select {
		case outcome := <-result:
			if release {
				if err := releaseFault(outcome); err != nil {
					s.fail(err)
				}
			}
		case <-timer.C:
			s.fail(ErrCancellationDeadline)
		}
	}()
	for {
		if ctx.Err() != nil {
			panic(http.ErrAbortHandler)
		}
		select {
		case <-ctx.Done():
			panic(http.ErrAbortHandler)
		case record, ok := <-output.records:
			if ctx.Err() != nil {
				panic(http.ErrAbortHandler)
			}
			if !ok {
				outcome := <-result
				joined = true
				if outcome.panicked {
					if release {
						s.fail(releaseFault(outcome))
					}
					panic(http.ErrAbortHandler)
				}
				if errors.Is(outcome.err, ErrCancelled) {
					panic(http.ErrAbortHandler)
				}
				completion := outcome.completion
				if outcome.err != nil {
					completion = Completion{ExitCode: 1, Error: &CommandError{Code: "command_failed", Message: outcome.err.Error()}}
				}
				if release {
					if fault := releaseFault(handlerResult{completion: completion}); fault != nil {
						defer s.fail(fault)
					}
				}
				record = Record{Kind: Completed, Completion: completion}
			}
			encoded, err := record.Encode()
			if err != nil {
				panic(http.ErrAbortHandler)
			}
			if _, err = w.Write(encoded); err != nil {
				return
			}
			if err = controller.Flush(); err != nil {
				return
			}
			if !ok {
				return
			}
		}
	}
}

// singleListener gives net/http exactly the connection supplied by its owner.
type singleListener struct {
	conn   net.Conn
	taken  bool
	closed chan struct{}
	once   sync.Once
}

func (l *singleListener) Accept() (net.Conn, error) {
	if !l.taken {
		l.taken = true
		return l.conn, nil
	}
	<-l.closed
	return nil, net.ErrClosed
}

// Close stops accepting connections.
func (l *singleListener) Close() error {
	l.once.Do(func() { close(l.closed) })
	return nil
}

func (l *singleListener) Addr() net.Addr { return l.conn.LocalAddr() }

// observedConn distinguishes a peer closing from net/http rejecting its protocol.
type observedConn struct {
	net.Conn
	peerClosed atomic.Bool
}

func (c *observedConn) Read(buffer []byte) (int, error) {
	count, err := c.Conn.Read(buffer)
	if err != nil && !errors.Is(err, net.ErrClosed) {
		var timeout net.Error
		if !errors.As(err, &timeout) || !timeout.Timeout() {
			c.peerClosed.Store(true)
		}
	}
	return count, err
}

// Serve owns conn on every return path, drains admitted calls, and releases the
// handler. ErrCancellationDeadline requires the process to exit: Go cannot stop
// an uncooperative handler or Closer, but no SDK join goroutine remains behind.
func Serve(owner context.Context, conn net.Conn, handler Handler) error {
	// The service owns the transport even when catalog validation fails.
	// Close only releases IO; the selected protocol or handler result owns the error.
	defer conn.Close()
	info := ServiceInfo{ProtocolVersion: Version, Operations: slices.Clone(handler.Operations())}
	data, err := Encode(info)
	if err != nil {
		return err
	}
	if len(data) > MaxMetadataBytes {
		return ErrTooLarge
	}
	ctx, cancel := context.WithCancel(owner)
	defer cancel()
	observed := &observedConn{Conn: conn}
	listener := &singleListener{conn: observed, closed: make(chan struct{})}
	s := &service{
		handler:  handler,
		info:     info,
		ctx:      ctx,
		cancel:   cancel,
		numbers:  newNumbers(),
		fault:    make(chan error, 1),
		shutdown: make(chan struct{}),
		requests: make(map[chan struct{}]struct{}),
	}

	s.server = &http.Server{
		Handler:           s,
		Protocols:         protocols(),
		HTTP2:             http2Config(),
		MaxHeaderBytes:    HeaderListBytes - httpHeaderAdjustment,
		ReadHeaderTimeout: phaseTimeout,
		BaseContext:       func(net.Listener) context.Context { return ctx },
		ConnState: func(_ net.Conn, state http.ConnState) {
			if state == http.StateClosed {
				// Closing the synthetic listener wakes Serve; it has no failure path.
				_ = listener.Close()
			}
		},
	}
	served := make(chan error, 1)
	go func() { served <- s.server.Serve(listener) }()
	serverJoined := false
	select {
	case <-s.shutdown:
		// Shutdown's own goroutine is joined before this function proceeds.
		drained := make(chan error, 1)
		go func() { drained <- s.server.Shutdown(ctx) }()
		select {
		case err = <-drained:
		case <-ctx.Done():
			// Closing is best effort because cancellation or a retained fault owns the outcome.
			_ = s.server.Close()
			<-drained
		}
	case serveErr := <-served:
		serverJoined = true
		if !errors.Is(serveErr, net.ErrClosed) && !errors.Is(serveErr, http.ErrServerClosed) {
			err = serveErr
		}
		s.admission.Lock()
		if ctx.Err() == nil && s.state == serviceRunning && !observed.peerClosed.Load() {
			err = errors.New("command service connection failed")
		}
		s.admission.Unlock()
	case <-ctx.Done():
	}
	cancel()
	s.numbers.stop()
	// Stop admission and interrupt all HTTP IO before joining requests.
	_ = s.server.Close()
	if !serverJoined {
		<-served
	}
	s.admission.Lock()
	s.state = serviceStopped
	pending := make([]chan struct{}, 0, len(s.requests))
	for done := range s.requests {
		pending = append(pending, done)
	}
	s.admission.Unlock()
	// Each request joins its handler with the grace; no indefinite WaitGroup waiter exists.
	for _, done := range pending {
		<-done
	}
	select {
	case fault := <-s.fault:
		err = fault
	default:
	}
	if closer, ok := handler.(Closer); ok {
		closeCtx, closeCancel := context.WithTimeout(context.Background(), cancellationGrace)
		closed := make(chan error, 1)
		go func() { closed <- closer.Close(closeCtx) }()
		select {
		case closeErr := <-closed:
			if closeErr != nil {
				err = closeErr
			}
		case <-closeCtx.Done():
			err = ErrCancellationDeadline
		}
		closeCancel()
	}
	return err
}
