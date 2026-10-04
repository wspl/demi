package commandsdk

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/http"
	"slices"
	"sync"
	"time"

	"github.com/wspl/demi/internal/commandproto"
)

// ErrCancellationDeadline requires the owner to retire the service process.
var ErrCancellationDeadline = errors.New("handler exceeded cancellation deadline; retire the service process")

// ErrConversationCleanup means a conversation release failed; the owner
// retires the service process.
var ErrConversationCleanup = errors.New("conversation cleanup failed; retire the service process")

// InvocationContext contains explicit invocation state; process cwd and environment never change.
type InvocationContext[M commandproto.Metadata] struct {
	Request M
	Input   *Input
	Output  *Output
}

// ConversationContext carries trusted lifecycle metadata without stdin or environment.
type ConversationContext struct {
	Request commandproto.ConversationRequest
	Output  *Output
}

// Handler owns invocation work and cooperates with context cancellation.
// Optional Conversation, Close, SetNumbers and SetArtifacts methods supply lifecycle hooks.
type Handler[M commandproto.Metadata] interface {
	Operations() []string
	Invoke(context.Context, InvocationContext[M]) (commandproto.Completion, error)
}

// Serve serves native invocation metadata on one owned connection.
func Serve(ctx context.Context, conn net.Conn, h Handler[commandproto.Invocation]) error {
	return serve(
		ctx,
		conn,
		h,
		commandproto.DecodeInvocation,
		func(m commandproto.Invocation) string {
			return m.Operation
		},
	)
}

// ServeLocal serves the local command client's distinct metadata contract.
func ServeLocal(ctx context.Context, conn net.Conn, h Handler[commandproto.LocalInvocation]) error {
	return serve(
		ctx,
		conn,
		h,
		commandproto.DecodeLocalInvocation,
		func(m commandproto.LocalInvocation) string {
			return m.Operation
		},
	)
}

type oneListener struct {
	conn      net.Conn
	once      sync.Once
	closed    chan struct{}
	closeOnce sync.Once
}

// Accept yields the owned connection once, then waits for closure.
func (l *oneListener) Accept() (net.Conn, error) {
	var c net.Conn
	l.once.Do(func() {
		c = l.conn
	})
	if c != nil {
		return c, nil
	}
	<-l.closed
	return nil, net.ErrClosed
}

// Close wakes any pending accept without closing the connection.
func (l *oneListener) Close() error {
	l.closeOnce.Do(func() {
		close(l.closed)
	})
	return nil
}

// Addr returns the owned connection's local address.
func (l *oneListener) Addr() net.Addr {
	return l.conn.LocalAddr()
}

type observedConn struct {
	net.Conn
	done chan struct{}
	once sync.Once
}

// Close releases the owned transport.
func (c *observedConn) Close() error {
	err := c.Conn.Close()
	c.once.Do(func() {
		close(c.done)
	})
	return err
}

// serve owns the connection, handler hooks and service lifecycle.
func serve[M commandproto.Metadata](
	ctx context.Context,
	conn net.Conn,
	h Handler[M],
	decode func([]byte) (M, error),
	operation func(M) string,
) error {
	info, catalog, err := serviceCatalog(h)
	if err != nil {
		return errors.Join(err, conn.Close())
	}
	owner, cancel := context.WithCancel(ctx)
	defer cancel()
	streams, finish := context.WithCancel(owner)
	defer finish()
	numbers, draws := NumbersChannel()
	defer numbers.Close()
	artifacts, asks := ArtifactsChannel()
	defer artifacts.Close()
	configureService(h, numbers, artifacts)
	watched := &observedConn{Conn: conn, done: make(chan struct{})}
	listener := &oneListener{conn: watched, closed: make(chan struct{})}
	shutdown := make(chan struct{})
	fatal := make(chan error, 1)
	var mu sync.Mutex
	phase := serviceAccepting
	opened := map[string]bool{}
	var calls sync.WaitGroup
	var work sync.WaitGroup
	fail := func(e error) {
		select {
		case fatal <- e:
			cancel()
		default:
		}
	}
	server := serviceServer(owner)
	requests := serviceRequests[M]{
		h: h, decode: decode, operation: operation, info: info, catalog: catalog,
		mu: &mu, phase: &phase, opened: opened, calls: &calls, work: &work,
		shutdown: shutdown, streams: streams, numbers: numbers, draws: draws,
		artifacts: artifacts, asks: asks, fail: fail,
	}
	server.Handler = http.HandlerFunc(requests.serveHTTP)
	serving := make(chan error, 1)
	go func() {
		serving <- server.Serve(listener)
	}()
	var outcome error
	closeHandler := sync.OnceFunc(func() {
		outcome = closeServiceHandler(owner, h, outcome)
	})
	requests.waitForShutdown(ctx, owner, server, watched.done, fatal, finish, closeHandler, &outcome)
	return requests.stop(owner, cancel, finish, server, serving, &outcome, closeHandler, fatal)
}

// serviceServer is the HTTP/2 server of one service connection, whose
// requests start from owner.
func serviceServer(owner context.Context) *http.Server {
	return &http.Server{
		Protocols:         protocols(),
		HTTP2:             h2Config(false),
		MaxHeaderBytes:    16 * 1024,
		ReadHeaderTimeout: phaseTimeout,
		BaseContext: func(net.Listener) context.Context {
			return owner
		},
	}
}

type servicePhase uint8

const (
	serviceAccepting servicePhase = iota
	serviceDraining
	serviceStopped
)

type invocationResult struct {
	cleanup error
	err     error
	aborted bool
}
type handlerResult struct {
	completion commandproto.Completion
	err        error
	panicked   bool
}

// runInvocation joins handler work after forwarding its bounded output.
func runInvocation(
	ctx context.Context,
	r *http.Request,
	w http.ResponseWriter,
	remaining int64,
	invoke func(context.Context, *Input, *Output) (commandproto.Completion, error),
) invocationResult {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	o, records := OutputChannel(ctx)
	i := NewInput(&httpInput{body: r.Body, output: o, remaining: remaining})
	done := make(chan handlerResult, 1)
	go func() {
		result := handlerResult{}
		defer func() {
			if p := recover(); p != nil {
				result.err = fmt.Errorf("command handler panicked: %v", p)
				result.panicked = true
			}
			close(o.records)
			done <- result
		}()
		result.completion, result.err = invoke(ctx, i, o)
	}()
	write := func(record commandproto.Record) error {
		b, err := commandproto.EncodeRecord(record)
		if err != nil {
			return err
		}
		if _, err = w.Write(b); err != nil {
			return err
		}
		return http.NewResponseController(w).Flush()
	}
	failure := forwardRecords(ctx, records, write)
	if failure != nil {
		cancel()
		return cancelInvocation(ctx, r, done, failure)
	}
	result := <-done
	if result.panicked {
		return invocationResult{err: result.err, cleanup: result.err, aborted: true}
	}
	if errors.Is(result.err, context.Canceled) {
		return invocationResult{err: result.err, aborted: true}
	}
	if result.err != nil {
		result.completion = commandproto.Completion{
			ExitCode: 1,
			Error:    &commandproto.CommandError{Code: "command_failed", Message: result.err.Error()},
		}
	}
	err := write(commandproto.Completed{Completion: result.completion})
	if result.completion.ExitCode != 0 && result.err == nil {
		result.err = fmt.Errorf("release exited with status %d", result.completion.ExitCode)
	}
	return invocationResult{err: errors.Join(result.err, err), cleanup: result.err, aborted: err != nil}
}

// serviceCatalog validates and encodes the bounded service catalog.
func serviceCatalog[M commandproto.Metadata](h Handler[M]) (commandproto.ServiceInfo, []byte, error) {
	info := commandproto.ServiceInfo{ProtocolVersion: commandproto.Version, Operations: h.Operations()}
	if err := info.Validate(); err != nil {
		return info, nil, err
	}
	catalog, err := info.MarshalJSON()
	if err != nil {
		return info, nil, err
	}
	if len(catalog) > commandproto.MaxMetadataBytes {
		return info, nil, commandproto.ErrTooLarge
	}
	return info, catalog, nil
}

// serviceRequests bundles the existing service state used by HTTP request handling.
type serviceRequests[M commandproto.Metadata] struct {
	h           Handler[M]
	decode      func([]byte) (M, error)
	operation   func(M) string
	info        commandproto.ServiceInfo
	catalog     []byte
	mu          *sync.Mutex
	phase       *servicePhase
	opened      map[string]bool
	calls, work *sync.WaitGroup
	shutdown    chan struct{}
	streams     context.Context
	numbers     *Numbers
	draws       <-chan Draw
	artifacts   *Artifacts
	asks        <-chan ArtifactPending
	fail        func(error)
}

func (s serviceRequests[M]) serveHTTP(w http.ResponseWriter, r *http.Request) {
	s.mu.Lock()
	if *s.phase == serviceStopped {
		s.mu.Unlock()
		w.WriteHeader(http.StatusServiceUnavailable)
		return
	}
	s.calls.Add(1)
	defer s.calls.Done()
	stopped := *s.phase != serviceAccepting
	if r.Method == http.MethodPost && r.URL.Path == commandproto.ShutdownPath {
		if *s.phase != serviceDraining {
			*s.phase = serviceDraining
			close(s.shutdown)
		}
		s.mu.Unlock()
		w.WriteHeader(http.StatusOK)
		return
	}
	duplicate := s.opened[r.URL.Path]
	stream := r.Method == http.MethodPost &&
		(r.URL.Path == commandproto.NumbersPath || r.URL.Path == commandproto.ArtifactsPath)
	if stream {
		s.opened[r.URL.Path] = true
	} else if !stopped {
		s.work.Add(1)
		defer s.work.Done()
	}
	s.mu.Unlock()
	s.respond(w, r, stopped, duplicate, stream)
}

// respond dispatches an admitted request after the service mutex is released.
func (s serviceRequests[M]) respond(w http.ResponseWriter, r *http.Request, stopped, duplicate, stream bool) {
	if stopped {
		w.WriteHeader(http.StatusServiceUnavailable)
		return
	}
	if r.Method == http.MethodGet && r.URL.Path == commandproto.InfoPath {
		_, _ = w.Write(s.catalog)
		return
	}
	if r.Method != http.MethodPost ||
		(r.URL.Path != commandproto.InvokePath && r.URL.Path != commandproto.ConversationPath && !stream) {
		w.WriteHeader(http.StatusNotFound)
		return
	}
	rc := http.NewResponseController(w)
	if err := rc.SetReadDeadline(time.Now().Add(phaseTimeout)); err != nil {
		panic(http.ErrAbortHandler)
	}
	b, err := readChunk(r.Body, commandproto.MaxMetadataBytes)
	if clearErr := rc.SetReadDeadline(time.Time{}); err == nil {
		err = clearErr
	}
	if err != nil {
		w.WriteHeader(http.StatusBadRequest)
		return
	}
	invoke, release, valid := s.invocation(w, r, b, duplicate)
	if !valid {
		return
	}
	w.WriteHeader(http.StatusOK)
	if err := rc.Flush(); err != nil {
		return
	}
	remaining := r.ContentLength
	if remaining >= 0 {
		remaining -= int64(4 + len(b))
	}
	result := runInvocation(r.Context(), r, w, remaining, invoke)
	if release && result.cleanup != nil {
		s.fail(fmt.Errorf("%w: %w", ErrConversationCleanup, result.cleanup))
	}
	if errors.Is(result.err, ErrCancellationDeadline) {
		s.fail(result.err)
	}
	if result.aborted {
		panic(http.ErrAbortHandler)
	}
}

// invocation selects the validated command or service-stream handler.
func (s serviceRequests[M]) invocation(
	w http.ResponseWriter,
	r *http.Request,
	b []byte,
	duplicate bool,
) (func(context.Context, *Input, *Output) (commandproto.Completion, error), bool, bool) {
	var invoke func(context.Context, *Input, *Output) (commandproto.Completion, error)
	release := false
	switch r.URL.Path {
	case commandproto.InvokePath:
		m, e := s.decode(b)
		if e != nil {
			w.WriteHeader(http.StatusBadRequest)
			return nil, false, false
		}
		if !slices.Contains(s.info.Operations, s.operation(m)) {
			w.WriteHeader(http.StatusNotFound)
			return nil, false, false
		}
		invoke = func(ctx context.Context, i *Input, o *Output) (commandproto.Completion, error) {
			return s.h.Invoke(ctx, InvocationContext[M]{Request: m, Input: i, Output: o})
		}
	case commandproto.ConversationPath:
		m, e := commandproto.DecodeConversationRequest(b)
		if e != nil {
			w.WriteHeader(http.StatusBadRequest)
			return nil, false, false
		}
		_, release = m.(*commandproto.ConversationRelease)
		invoke = func(ctx context.Context, _ *Input, o *Output) (commandproto.Completion, error) {
			return s.conversation(ctx, m, o)
		}
	default:
		if _, e := commandproto.DecodeStreamOpen(b); e != nil {
			w.WriteHeader(http.StatusBadRequest)
			return nil, false, false
		}
		if duplicate {
			w.WriteHeader(http.StatusConflict)
			return nil, false, false
		}
		if r.URL.Path == commandproto.NumbersPath {
			invoke = func(ctx context.Context, i *Input, o *Output) (commandproto.Completion, error) {
				return relayNumbers(ctx, s.streams, i, o, s.numbers, s.draws)
			}
		} else {
			invoke = func(ctx context.Context, i *Input, o *Output) (commandproto.Completion, error) {
				return relayArtifacts(ctx, s.streams, i, o, s.artifacts, s.asks)
			}
		}
	}
	return invoke, release, true
}

// conversation invokes the lifecycle hook or writes its default response.
func (s serviceRequests[M]) conversation(
	ctx context.Context,
	m commandproto.ConversationRequest,
	o *Output,
) (commandproto.Completion, error) {
	if hook, ok := any(s.h).(interface {
		Conversation(context.Context, ConversationContext) (commandproto.Completion, error)
	}); ok {
		return hook.Conversation(ctx, ConversationContext{Request: m, Output: o})
	}
	body := []byte(`{}`)
	if _, ok := m.(*commandproto.ConversationQuery); ok {
		body = []byte(`{"conversations":[]}`)
	}
	return commandproto.Completion{}, o.Stdout(ctx, body)
}

// closeServiceHandler waits for bounded handler cleanup and recovers hook panics.
func closeServiceHandler[M commandproto.Metadata](_ context.Context, h Handler[M], outcome error) error {
	if hook, ok := any(h).(interface{ Close(context.Context) error }); ok {
		closing, cancelClose := context.WithTimeout(context.Background(), cancelTimeout)
		defer cancelClose()
		done := make(chan error, 1)
		go func() {
			var closeErr error
			defer func() {
				if p := recover(); p != nil {
					closeErr = fmt.Errorf("command close panicked: %v", p)
				}
				done <- closeErr
			}()
			closeErr = hook.Close(closing)
		}()
		select {
		case e := <-done:
			outcome = errors.Join(outcome, e)
		case <-closing.Done():
			outcome = errors.Join(outcome, ErrCancellationDeadline)
		}
	}
	return outcome
}

// forwardRecords flushes handler output until cancellation, failure or completion.
func forwardRecords(
	ctx context.Context,
	records <-chan commandproto.Record,
	write func(commandproto.Record) error,
) error {
	var failure error
loop:
	for {
		select {
		case <-ctx.Done():
			failure = ctx.Err()
			break loop
		case record, ok := <-records:
			if !ok {
				break loop
			}
			if err := write(record); err != nil {
				failure = err
				break loop
			}
		}
	}
	return failure
}

// cancelInvocation interrupts input and waits for bounded invocation cleanup.
func cancelInvocation(_ context.Context, r *http.Request, done <-chan handlerResult, failure error) invocationResult {
	// Closing the request body interrupts a handler blocked reading stdin.
	_ = r.Body.Close()
	timer := time.NewTimer(cancelTimeout)
	defer timer.Stop()
	select {
	case result := <-done:
		if result.err != nil && !errors.Is(result.err, context.Canceled) {
			return invocationResult{err: result.err, cleanup: result.err, aborted: true}
		}
		if result.completion.ExitCode != 0 {
			err := fmt.Errorf("release exited with status %d", result.completion.ExitCode)
			return invocationResult{err: err, cleanup: err, aborted: true}
		}
		return invocationResult{err: failure, aborted: true}
	case <-timer.C:
		return invocationResult{err: ErrCancellationDeadline, aborted: true}
	}
}

// stop cancels and joins the service before performing any remaining handler cleanup.
func (s serviceRequests[M]) stop(
	_ context.Context,
	cancel, finish context.CancelFunc,
	server *http.Server,
	serving <-chan error,
	outcome *error,
	closeHandler func(),
	fatal <-chan error,
) error {
	cancel()
	finish()
	s.mu.Lock()
	*s.phase = serviceStopped
	s.mu.Unlock()
	closeErr := server.Close()
	serveErr := <-serving
	s.calls.Wait()
	if !errors.Is(serveErr, http.ErrServerClosed) {
		*outcome = errors.Join(*outcome, serveErr)
	}
	*outcome = errors.Join(*outcome, closeErr)
	closeHandler()
	select {
	case e := <-fatal:
		*outcome = errors.Join(*outcome, e)
	default:
	}
	return *outcome
}

// configureService supplies the runner-backed sources to optional handler hooks.
func configureService[M commandproto.Metadata](h Handler[M], numbers *Numbers, artifacts *Artifacts) {
	if hook, ok := any(h).(interface{ SetNumbers(*Numbers) }); ok {
		hook.SetNumbers(numbers)
	}
	if hook, ok := any(h).(interface{ SetArtifacts(*Artifacts) }); ok {
		hook.SetArtifacts(artifacts)
	}
}

// waitForShutdown drains command work on shutdown or waits for the service owner to end.
func (s serviceRequests[M]) waitForShutdown(
	ctx, owner context.Context,
	server *http.Server,
	disconnected <-chan struct{},
	fatal <-chan error,
	finish context.CancelFunc,
	closeHandler func(),
	outcome *error,
) {
	select {
	case <-s.shutdown:
		// Drain commands before closing the handler, retaining its runner streams
		// until cleanup finishes. HTTP shutdown can then drain those streams too.
		s.work.Wait()
		closeHandler()
		finish()
		*outcome = errors.Join(*outcome, server.Shutdown(owner))
	case <-ctx.Done():
	case <-disconnected:
	case *outcome = <-fatal:
	}
}
