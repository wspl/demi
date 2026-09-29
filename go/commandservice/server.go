package commandservice

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"math"
	"net"
	"net/http"
	"os"
	"runtime/debug"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

// errBrokenConnection is how Serve ends when the HTTP/2 connection failed: the
// peer broke the protocol and the server closed the connection, or the peer
// closed it before the handshake was complete.
var errBrokenConnection = errors.New("the HTTP/2 connection failed")

// Serve serves one connection until the peer closes it, a drain that the peer
// asked for with a shutdown request completes, or ctx ends. It closes conn and
// returns only after every call has stopped and the handler's Close, if it has
// one, has returned.
//
// It returns nil when the service ended normally, also when the peer closed its
// side after an answered shutdown. Otherwise the service is faulty, and its
// process must exit so that the runner retires it:
//   - [ErrCancellationDeadline]: a call, or Close, did not stop within the
//     cancellation grace.
//   - [ErrConversationCleanup]: a conversation release failed.
//   - An error of the connection: the peer broke the HTTP/2 protocol, so that the
//     server closed the connection while the service was neither draining, nor
//     cancelled, nor told by the peer that it closed; or the transport itself
//     failed.
//   - An [*InvalidError], before Serve serves anything: the handler's catalog
//     breaks the rules of the wire.
//
// A handshake that fails, times out ([ErrHandshakeTimeout]) or is cut short by
// ctx ([ErrCancelled]) ends Serve without calling Close: nothing was served.
func Serve(ctx context.Context, conn net.Conn, handler Handler) error {
	s, err := newService(ctx, handler)
	if err != nil {
		// The connection is given up; there is nothing to say to its peer.
		_ = conn.Close()
		return err
	}
	return s.serve(conn)
}

// unencryptedHTTP2 is the protocol set of both ends: HTTP/2 with prior
// knowledge, without TLS.
func unencryptedHTTP2() *http.Protocols {
	var protocols http.Protocols
	protocols.SetUnencryptedHTTP2(true)
	return &protocols
}

// A service is one connection's server: it answers the wire's requests, runs
// each call on a goroutine of its own, and counts them.
type service struct {
	handler      Handler
	conversation func(*ConversationCall) (Completion, error)
	info         []byte
	operations   map[string]bool
	numbers      *numberSource
	srv          *http.Server

	// ctx ends when the service stops, or when the context Serve was given
	// ends; every call's context descends from it.
	ctx      context.Context
	stop     context.CancelFunc
	draining atomic.Bool
	// handshake limits the time the peer has to complete the HTTP/2 handshake,
	// which handshaken says it did.
	handshake  *time.Timer
	handshaken atomic.Bool

	// callsMu guards stopped and the additions to calls.
	callsMu sync.Mutex
	stopped bool
	calls   sync.WaitGroup

	faultOnce sync.Once
	faultErr  error
	faulted   chan struct{}
}

// newService returns the service of handler, or the error of the handler's
// catalog.
func newService(ctx context.Context, handler Handler) (*service, error) {
	catalog := ServiceInfo{ProtocolVersion: Version, Operations: handler.Operations()}
	info, err := Encode(catalog)
	if err != nil {
		return nil, err
	}
	s := &service{
		handler:      handler,
		conversation: defaultConversation,
		info:         info,
		operations:   map[string]bool{},
		numbers:      newNumberSource(),
		faulted:      make(chan struct{}),
	}
	s.ctx, s.stop = context.WithCancel(ctx)
	for _, operation := range catalog.Operations {
		s.operations[operation] = true
	}
	if conversations, ok := handler.(ConversationHandler); ok {
		s.conversation = conversations.Conversation
	}
	return s, nil
}

// What ended a service's wait: its context, a fault, or the connection.
type endCause int

const (
	endedByContext endCause = iota
	endedByFault
	endedByConnection
)

// serve serves conn until the service stops, and then releases everything it
// holds; [Serve] documents how it returns.
func (s *service) serve(conn net.Conn) error {
	defer s.stop()
	transport := newServiceConn(conn)
	s.handshake = time.AfterFunc(handshakeTimeout, func() { s.fail(ErrHandshakeTimeout) })
	defer s.handshake.Stop()
	s.srv = &http.Server{
		Handler:   s,
		Protocols: unencryptedHTTP2(),
		// Invocations are not counted (docs/execution/native-runtime.md
		// § Validation and flow control): each stream has its own window and
		// output queue, and the connection's window is the largest HTTP/2
		// allows, so the connection never becomes the constraint.
		HTTP2: &http.HTTP2Config{
			MaxConcurrentStreams:          math.MaxInt32,
			MaxReceiveBufferPerStream:     MaxRecordBytes,
			MaxReceiveBufferPerConnection: maxHTTP2Window,
		},
		MaxHeaderBytes: maxHeaderBytes,
		BaseContext:    func(net.Listener) context.Context { return s.ctx },
		ConnState:      s.connState,
	}
	serving := make(chan struct{})
	go func() {
		// After Close, Serve returns http.ErrServerClosed; what broke the
		// connection is what serviceConn kept.
		_ = s.srv.Serve(newConnListener(transport))
		close(serving)
	}()

	var cause endCause
	select {
	case <-s.ctx.Done():
		cause = endedByContext
	case <-s.faulted:
		cause = endedByFault
	case <-transport.closed:
		cause = endedByConnection
	}
	handshaken := s.handshaken.Load()

	s.stop()
	// Close reports only how the listener closed, which the listener owns.
	_ = s.srv.Close()
	<-serving
	s.awaitCalls()
	s.numbers.end()
	if !handshaken {
		// Nothing was served, so there is nothing to release.
		return s.failedHandshake(cause)
	}
	if err := s.closeHandler(); err != nil {
		return err
	}
	if err := s.fault(); err != nil {
		return err
	}
	draining := s.draining.Load()
	if failure := transport.failure(); failure != nil && !(draining && peerClosed(failure)) {
		return failure
	}
	if cause == endedByConnection && !draining && !transport.peerEnded() {
		return errBrokenConnection
	}
	return nil
}

// failedHandshake returns how Serve ends when the handshake was not complete:
// it timed out, ctx ended, or the connection closed.
func (s *service) failedHandshake(cause endCause) error {
	if err := s.fault(); err != nil {
		return err
	}
	if cause == endedByContext {
		return ErrCancelled
	}
	return errBrokenConnection
}

// connState watches the connection for the end of its handshake: the HTTP/2
// server reports the connection active once it has read the client's preface.
func (s *service) connState(_ net.Conn, state http.ConnState) {
	if state == http.StateActive {
		s.handshaken.Store(true)
		s.handshake.Stop()
	}
}

// fail records the service's first fault and wakes Serve, which retires the
// service.
func (s *service) fail(err error) {
	s.faultOnce.Do(func() {
		s.faultErr = err
		close(s.faulted)
	})
}

// fault returns the service's first fault, if it has one.
func (s *service) fault() error {
	select {
	case <-s.faulted:
		return s.faultErr
	default:
		return nil
	}
}

// begin admits a request, unless the service has stopped taking them.
func (s *service) begin() bool {
	s.callsMu.Lock()
	defer s.callsMu.Unlock()
	if s.stopped {
		return false
	}
	s.calls.Add(1)
	return true
}

// awaitCalls refuses every later request and waits for the ones running. Each
// ends within the cancellation grace of the service's stop.
func (s *service) awaitCalls() {
	s.callsMu.Lock()
	s.stopped = true
	s.callsMu.Unlock()
	s.calls.Wait()
}

// closeHandler calls the handler's Close, if it has one, once every call has
// stopped, with a deadline of the cancellation grace.
func (s *service) closeHandler() error {
	closer, ok := s.handler.(Closer)
	if !ok {
		return nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), cancellationGrace)
	defer cancel()
	closed := make(chan error, 1)
	go func() { closed <- closer.Close(ctx) }()
	select {
	case err := <-closed:
		return err
	case <-ctx.Done():
		return ErrCancellationDeadline
	}
}

// defaultConversation answers the conversation endpoint of a handler that
// holds no conversation state.
func defaultConversation(call *ConversationCall) (Completion, error) {
	answer := "{}"
	if _, status := call.Request.(StatusRequest); status {
		answer = `{"conversations":[]}`
	}
	if _, err := io.WriteString(call.Stdout, answer); err != nil {
		return Completion{}, err
	}
	return Completion{}, nil
}

// ServeHTTP answers one request of the wire. After a shutdown the HTTP/2
// server refuses the streams above its GOAWAY's last stream ID itself; the
// requests it had already accepted are answered 503 here.
func (s *service) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	if !s.begin() {
		w.WriteHeader(http.StatusServiceUnavailable)
		return
	}
	defer s.calls.Done()
	// The wire's responses carry no headers, and net/http would sniff a
	// Content-Type from the body.
	w.Header()["Content-Type"] = nil
	// The route is the path as the peer wrote it, still escaped: a request for
	// /v1/%69nfo is not one for /v1/info.
	route := r.Method + " " + r.URL.EscapedPath()
	if route == "POST "+ShutdownPath {
		s.shutdown(w)
		return
	}
	if s.draining.Load() {
		w.WriteHeader(http.StatusServiceUnavailable)
		return
	}
	switch route {
	case "GET " + InfoPath:
		s.serveInfo(w)
	case "POST " + InvokePath:
		s.serveCall(w, r, invocationCall)
	case "POST " + ConversationPath:
		s.serveCall(w, r, conversationCall)
	case "POST " + NumbersPath:
		s.serveCall(w, r, numbersCall)
	default:
		w.WriteHeader(http.StatusNotFound)
	}
}

func (s *service) serveInfo(w http.ResponseWriter) {
	w.WriteHeader(http.StatusOK)
	// A write fails only when the peer is gone, which nothing here answers.
	_, _ = w.Write(s.info)
}

// shutdown stops admission and drains: every new request is answered 503, the
// calls running finish, and the connection closes once they have. The numbers
// stream, which would hold the connection open, completes.
func (s *service) shutdown(w http.ResponseWriter) {
	s.draining.Store(true)
	s.numbers.finish()
	w.WriteHeader(http.StatusOK)
	// The server sends GOAWAY and closes the connection once its streams end.
	// Serve waits for this goroutine as it does for the calls: the goroutine
	// ends when the connection is closed or the service stops, and its error is
	// only that of the context.
	s.calls.Add(1)
	go func() {
		defer s.calls.Done()
		_ = s.srv.Shutdown(s.ctx)
	}()
}

// The kinds of request that run something.
type callKind int

const (
	invocationCall callKind = iota
	conversationCall
	numbersCall
)

// A pendingCall is a call whose metadata the service accepted.
type pendingCall struct {
	// release says the call releases a conversation, so that a failure of it
	// faults the service.
	release bool
	run     func() (Completion, error)
}

// A result is how a handler returned.
type result struct {
	completion Completion
	err        error
}

// serveCall runs one call: it reads the metadata, sends the response headers
// before waiting for input, runs the handler on a goroutine of its own, and
// writes the handler's records until it completes or the call is cancelled.
func (s *service) serveCall(w http.ResponseWriter, r *http.Request, kind callKind) {
	ctx, cancel := context.WithCancel(r.Context())
	defer cancel()
	state := newCallState(ctx, r.Body)
	pending, status := s.accept(r, kind, state)
	if status != 0 {
		w.WriteHeader(status)
		return
	}
	w.WriteHeader(http.StatusOK)
	controller := http.NewResponseController(w)
	if err := controller.Flush(); err != nil {
		// The caller is gone before the handler started.
		return
	}
	done := make(chan result, 1)
	go func() { done <- runHandler(pending.run) }()

	res, finished := s.pump(w, controller, state, done)
	if !finished {
		s.cancelled(cancel, state, done, pending.release)
		return
	}
	s.complete(w, controller, res, pending.release)
}

// accept reads the metadata of a request within the metadata timeout and
// checks it. It returns the HTTP status that refuses the request, or 0 with the
// call.
func (s *service) accept(r *http.Request, kind callKind, state *callState) (pendingCall, int) {
	document, err := readMetadata(r.Body)
	if err != nil {
		return pendingCall{}, http.StatusBadRequest
	}
	switch kind {
	case invocationCall:
		return s.acceptInvocation(document, state)
	case conversationCall:
		return s.acceptConversation(document, state)
	default:
		return s.acceptNumbers(document, state)
	}
}

func (s *service) acceptInvocation(document []byte, state *callState) (pendingCall, int) {
	invocation, err := Decode[Invocation](document)
	if err != nil {
		return pendingCall{}, http.StatusBadRequest
	}
	if !s.operations[invocation.Operation] {
		return pendingCall{}, http.StatusNotFound
	}
	call := &Call{
		Invocation: invocation,
		Stdin:      &Input{state: state},
		Stdout:     state.writer(RecordStdout),
		Stderr:     state.writer(RecordStderr),
		Numbers:    s.numbers.numbers(),
		state:      state,
	}
	return pendingCall{run: func() (Completion, error) { return s.handler.Invoke(call) }}, 0
}

func (s *service) acceptConversation(document []byte, state *callState) (pendingCall, int) {
	request, err := Decode[ConversationRequest](document)
	if err != nil {
		return pendingCall{}, http.StatusBadRequest
	}
	call := &ConversationCall{Request: request, Stdout: state.writer(RecordStdout), state: state}
	_, release := request.(ReleaseRequest)
	return pendingCall{
		release: release,
		run:     func() (Completion, error) { return s.conversation(call) },
	}, 0
}

func (s *service) acceptNumbers(document []byte, state *callState) (pendingCall, int) {
	if _, err := Decode[NumbersOpen](document); err != nil {
		return pendingCall{}, http.StatusBadRequest
	}
	// Only the first request opens the stream; a later one is refused once its
	// metadata is read.
	if !s.numbers.claim() {
		return pendingCall{}, http.StatusConflict
	}
	return pendingCall{run: func() (Completion, error) {
		return Completion{}, s.numbers.relay(state)
	}}, 0
}

// pump writes the call's records until its handler returns, when it returns the
// handler's result and true, or the call is cancelled: the caller reset the
// stream, the response failed, or the service is stopping.
func (s *service) pump(w http.ResponseWriter, controller *http.ResponseController, state *callState, done <-chan result) (result, bool) {
	for {
		if state.ctx.Err() != nil {
			return result{}, false
		}
		select {
		case record := <-state.records:
			if err := writeRecord(w, controller, record); err != nil {
				return result{}, false
			}
		case res := <-done:
			// The handler queued its records before it returned; they go
			// ahead of its completion.
			for {
				select {
				case record := <-state.records:
					if err := writeRecord(w, controller, record); err != nil {
						return result{}, false
					}
				default:
					return res, true
				}
			}
		case <-state.ctx.Done():
			return result{}, false
		}
	}
}

func writeRecord(w http.ResponseWriter, controller *http.ResponseController, record Record) error {
	data, err := record.MarshalBinary()
	if err != nil {
		return err
	}
	return writeBytes(w, controller, data)
}

// writeBytes sends data as part of the response, and flushes it, so that each
// record reaches the caller as it is written.
func writeBytes(w http.ResponseWriter, controller *http.ResponseController, data []byte) error {
	if _, err := w.Write(data); err != nil {
		return err
	}
	return controller.Flush()
}

// cancelled joins the handler of a call that was cancelled: the service stops
// reading its output, wakes it if it waits for input, and waits for it to
// return, at most the cancellation grace. A handler that does not return in
// time faults the service, since a handler that ignores its cancellation cannot
// be made to release its resources; so does a conversation release that fails.
// The stream is already reset, or its connection is closing, so nothing more is
// sent.
func (s *service) cancelled(cancel context.CancelFunc, state *callState, done <-chan result, release bool) {
	cancel()
	state.abort()
	timer := time.NewTimer(cancellationGrace)
	defer timer.Stop()
	select {
	case res := <-done:
		if release {
			if err := releaseFailure(res); err != nil {
				s.fail(err)
			}
		}
	case <-timer.C:
		s.fail(ErrCancellationDeadline)
	}
}

// releaseFailure returns the fault a cancelled release leaves: it panicked,
// failed for another reason than its cancellation, or ended with a nonzero
// exit code.
func releaseFailure(res result) error {
	switch {
	case res.err != nil && isCancellation(res.err):
		return nil
	case res.err != nil:
		return cleanupFailure(res.err)
	case res.completion.ExitCode != 0:
		return cleanupFailure(releaseExit(res.completion))
	}
	return nil
}

// releaseExit describes a release that ended with a nonzero exit code.
func releaseExit(completion Completion) error {
	if completion.Error == nil {
		return fmt.Errorf("release exited with status %d", completion.ExitCode)
	}
	return fmt.Errorf("release exited with status %d: %s: %s", completion.ExitCode, completion.Error.Code, completion.Error.Message)
}

func cleanupFailure(cause error) error {
	return fmt.Errorf("%w: %w", ErrConversationCleanup, cause)
}

// complete ends a call whose handler returned: it sends the completion, and
// leaves the service faulty when a conversation release failed. A handler that
// was cancelled or panicked has no completion to send: the stream is reset.
func (s *service) complete(w http.ResponseWriter, controller *http.ResponseController, res result, release bool) {
	var completion Completion
	var panicked *handlerPanic
	switch {
	case errors.As(res.err, &panicked):
		if release {
			s.fail(cleanupFailure(res.err))
		}
		panic(http.ErrAbortHandler)
	case res.err != nil && isCancellation(res.err):
		panic(http.ErrAbortHandler)
	case res.err != nil:
		completion = Completion{ExitCode: 1, Error: &CommandError{Code: "command_failed", Message: res.err.Error()}}
	default:
		completion = res.completion
	}
	if completion.Error != nil {
		// The text of an error can be any bytes, and JSON carries only UTF-8.
		fixed := *completion.Error
		fixed.Message = strings.ToValidUTF8(fixed.Message, "\uFFFD")
		completion.Error = &fixed
	}
	data, err := Record{Kind: RecordCompletion, Completion: completion}.MarshalBinary()
	if err != nil {
		// A completion that cannot be sent leaves only a reset to tell so.
		panic(http.ErrAbortHandler)
	}
	// A failed write means the caller is gone, which its stream shows.
	_ = writeBytes(w, controller, data)
	if release && completion.ExitCode != 0 {
		s.fail(cleanupFailure(releaseExit(completion)))
	}
}

// A handlerPanic is the error of a handler that panicked.
type handlerPanic struct {
	value any
}

func (p *handlerPanic) Error() string {
	return fmt.Sprintf("command handler panicked: %v", p.value)
}

// runHandler runs a handler, and turns its panic into an error: a handler that
// panics fails its own call, not the service.
func runHandler(run func() (Completion, error)) (res result) {
	defer func() {
		if value := recover(); value != nil {
			slog.Error("command handler panicked", "panic", value, "stack", string(debug.Stack()))
			res = result{err: &handlerPanic{value: value}}
		}
	}()
	completion, err := run()
	return result{completion: completion, err: err}
}

// peerClosed reports whether err is how a connection ends when its peer closes
// its side: a write into a closed pipe or socket, a reset, or an end of input.
// After an answered shutdown that is how a connection may end, on either side.
// Which errors say so depends on the platform ([closedByPeer]).
func peerClosed(err error) bool {
	return errors.Is(err, io.ErrUnexpectedEOF) || errors.Is(err, io.ErrClosedPipe) || closedByPeer(err)
}

// readMetadata reads the metadata that opens a request within the metadata
// timeout: a four-byte big-endian length, then that many bytes of JSON.
func readMetadata(body io.ReadCloser) ([]byte, error) {
	// A read that waits for the caller's metadata ends when the body closes.
	timer := time.AfterFunc(metadataTimeout, func() {
		// The body's close error says only that it was closed before.
		_ = body.Close()
	})
	document, err := readFrame(body, MaxMetadataBytes)
	if !timer.Stop() {
		// The time is up, and the body closed with it.
		return nil, os.ErrDeadlineExceeded
	}
	if errors.Is(err, io.EOF) {
		return nil, io.ErrUnexpectedEOF
	}
	return document, err
}
