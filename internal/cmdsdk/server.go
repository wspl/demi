package cmdsdk

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/http"
	"slices"
	"sync"
	"time"

	"github.com/wspl/demi/internal/commandwire"
)

// ErrCancellationDeadline requires the owner to retire the service process.
var ErrCancellationDeadline = errors.New("handler exceeded cancellation deadline; retire the service process")

// ConversationCleanupError requires retirement after failed conversation release.
type ConversationCleanupError struct{ Cause error }

func (e *ConversationCleanupError) Error() string {
	return "conversation cleanup failed; retire the service process: " + e.Cause.Error()
}
func (e *ConversationCleanupError) Unwrap() error { return e.Cause }

// InvocationContext contains explicit invocation state; process cwd and environment never change.
type InvocationContext[M commandwire.Metadata] struct {
	Request M
	Input   *Input
	Output  *Output
}

// ConversationContext carries trusted lifecycle metadata without stdin or environment.
type ConversationContext struct {
	Request commandwire.ConversationRequest
	Output  *Output
}

// Handler owns invocation work and cooperates with context cancellation.
// Optional Conversation, Close, SetNumbers and SetArtifacts methods supply lifecycle hooks.
type Handler[M commandwire.Metadata] interface {
	Operations() []string
	Invoke(context.Context, InvocationContext[M]) (commandwire.Completion, error)
}

// Serve serves native invocation metadata on one owned connection.
func Serve(ctx context.Context, conn net.Conn, h Handler[commandwire.Invocation]) error {
	return serve(ctx, conn, h, commandwire.DecodeInvocation, func(m commandwire.Invocation) string { return m.Operation })
}

// ServeLocal serves the local command client's distinct metadata contract.
func ServeLocal(ctx context.Context, conn net.Conn, h Handler[commandwire.LocalInvocation]) error {
	return serve(ctx, conn, h, commandwire.DecodeLocalInvocation, func(m commandwire.LocalInvocation) string { return m.Operation })
}

type oneListener struct {
	conn      net.Conn
	once      sync.Once
	closed    chan struct{}
	closeOnce sync.Once
}

func (l *oneListener) Accept() (net.Conn, error) {
	var c net.Conn
	l.once.Do(func() { c = l.conn })
	if c != nil {
		return c, nil
	}
	<-l.closed
	return nil, net.ErrClosed
}
func (l *oneListener) Close() error {
	l.closeOnce.Do(func() { close(l.closed) })
	return nil
}
func (l *oneListener) Addr() net.Addr { return l.conn.LocalAddr() }

type observedConn struct {
	net.Conn
	done chan struct{}
	once sync.Once
}

func (c *observedConn) Close() error {
	err := c.Conn.Close()
	c.once.Do(func() { close(c.done) })
	return err
}

func serve[M commandwire.Metadata](ctx context.Context, conn net.Conn, h Handler[M], decode func([]byte) (M, error), operation func(M) string) error {
	info := commandwire.ServiceInfo{ProtocolVersion: commandwire.Version, Operations: h.Operations()}
	if err := info.Validate(); err != nil {
		return errors.Join(err, conn.Close())
	}
	catalog, err := info.MarshalJSON()
	if err != nil {
		return errors.Join(err, conn.Close())
	}
	if len(catalog) > commandwire.MaxMetadataBytes {
		return errors.Join(commandwire.ErrTooLarge, conn.Close())
	}
	owner, cancel := context.WithCancel(ctx)
	defer cancel()
	streams, finish := context.WithCancel(owner)
	defer finish()
	numbers, draws := NumbersChannel()
	defer numbers.Close()
	artifacts, asks := ArtifactsChannel()
	defer artifacts.Close()
	if hook, ok := any(h).(interface{ SetNumbers(*Numbers) }); ok {
		hook.SetNumbers(numbers)
	}
	if hook, ok := any(h).(interface{ SetArtifacts(*Artifacts) }); ok {
		hook.SetArtifacts(artifacts)
	}
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
	server := &http.Server{Protocols: protocols(), HTTP2: h2Config(false), MaxHeaderBytes: 16 * 1024, ReadHeaderTimeout: phaseTimeout, BaseContext: func(net.Listener) context.Context { return owner }}
	server.Handler = http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		if phase == serviceStopped {
			mu.Unlock()
			w.WriteHeader(http.StatusServiceUnavailable)
			return
		}
		calls.Add(1)
		defer calls.Done()
		stopped := phase != serviceAccepting
		if r.Method == http.MethodPost && r.URL.Path == commandwire.ShutdownPath {
			if phase != serviceDraining {
				phase = serviceDraining
				close(shutdown)
			}
			mu.Unlock()
			w.WriteHeader(http.StatusOK)
			return
		}
		duplicate := opened[r.URL.Path]
		stream := r.Method == http.MethodPost && (r.URL.Path == commandwire.NumbersPath || r.URL.Path == commandwire.ArtifactsPath)
		if stream {
			opened[r.URL.Path] = true
		} else if !stopped {
			work.Add(1)
			defer work.Done()
		}
		mu.Unlock()
		if stopped {
			w.WriteHeader(http.StatusServiceUnavailable)
			return
		}
		if r.Method == http.MethodGet && r.URL.Path == commandwire.InfoPath {
			_, _ = w.Write(catalog)
			return
		}
		if r.Method != http.MethodPost || (r.URL.Path != commandwire.InvokePath && r.URL.Path != commandwire.ConversationPath && !stream) {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		rc := http.NewResponseController(w)
		if err := rc.SetReadDeadline(time.Now().Add(phaseTimeout)); err != nil {
			panic(http.ErrAbortHandler)
		}
		b, err := readChunk(r.Body, commandwire.MaxMetadataBytes)
		if clearErr := rc.SetReadDeadline(time.Time{}); err == nil {
			err = clearErr
		}
		if err != nil {
			w.WriteHeader(http.StatusBadRequest)
			return
		}
		var invoke func(context.Context, *Input, *Output) (commandwire.Completion, error)
		release := false
		switch r.URL.Path {
		case commandwire.InvokePath:
			m, e := decode(b)
			if e != nil {
				w.WriteHeader(http.StatusBadRequest)
				return
			}
			if !slices.Contains(info.Operations, operation(m)) {
				w.WriteHeader(http.StatusNotFound)
				return
			}
			invoke = func(ctx context.Context, i *Input, o *Output) (commandwire.Completion, error) {
				return h.Invoke(ctx, InvocationContext[M]{Request: m, Input: i, Output: o})
			}
		case commandwire.ConversationPath:
			m, e := commandwire.DecodeConversationRequest(b)
			if e != nil {
				w.WriteHeader(http.StatusBadRequest)
				return
			}
			_, release = m.(*commandwire.ConversationRelease)
			invoke = func(ctx context.Context, _ *Input, o *Output) (commandwire.Completion, error) {
				if hook, ok := any(h).(interface {
					Conversation(context.Context, ConversationContext) (commandwire.Completion, error)
				}); ok {
					return hook.Conversation(ctx, ConversationContext{Request: m, Output: o})
				}
				body := []byte(`{}`)
				if _, ok := m.(*commandwire.ConversationQuery); ok {
					body = []byte(`{"conversations":[]}`)
				}
				return commandwire.Completion{}, o.Stdout(ctx, body)
			}
		default:
			if _, e := commandwire.DecodeStreamOpen(b); e != nil {
				w.WriteHeader(http.StatusBadRequest)
				return
			}
			if duplicate {
				w.WriteHeader(http.StatusConflict)
				return
			}
			if r.URL.Path == commandwire.NumbersPath {
				invoke = func(ctx context.Context, i *Input, o *Output) (commandwire.Completion, error) {
					return relayNumbers(ctx, streams, i, o, numbers, draws)
				}
			} else {
				invoke = func(ctx context.Context, i *Input, o *Output) (commandwire.Completion, error) {
					return relayArtifacts(ctx, streams, i, o, artifacts, asks)
				}
			}
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
			fail(&ConversationCleanupError{Cause: result.cleanup})
		}
		if errors.Is(result.err, ErrCancellationDeadline) {
			fail(result.err)
		}
		if result.aborted {
			panic(http.ErrAbortHandler)
		}
	})
	serving := make(chan error, 1)
	go func() { serving <- server.Serve(listener) }()
	var outcome error
	closeHandler := sync.OnceFunc(func() {
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
	})
	select {
	case <-shutdown:
		// Drain commands before closing the handler, retaining its runner streams
		// until cleanup finishes. HTTP shutdown can then drain those streams too.
		work.Wait()
		closeHandler()
		finish()
		outcome = errors.Join(outcome, server.Shutdown(owner))
	case <-ctx.Done():
	case <-watched.done:
	case outcome = <-fatal:
	}
	cancel()
	finish()
	mu.Lock()
	phase = serviceStopped
	mu.Unlock()
	closeErr := server.Close()
	serveErr := <-serving
	calls.Wait()
	if !errors.Is(serveErr, http.ErrServerClosed) {
		outcome = errors.Join(outcome, serveErr)
	}
	outcome = errors.Join(outcome, closeErr)
	closeHandler()
	select {
	case e := <-fatal:
		outcome = errors.Join(outcome, e)
	default:
	}
	return outcome
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
	completion commandwire.Completion
	err        error
	panicked   bool
}

func runInvocation(ctx context.Context, r *http.Request, w http.ResponseWriter, remaining int64, invoke func(context.Context, *Input, *Output) (commandwire.Completion, error)) invocationResult {
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
	write := func(record commandwire.Record) error {
		b, err := commandwire.EncodeRecord(record)
		if err != nil {
			return err
		}
		if _, err = w.Write(b); err != nil {
			return err
		}
		return http.NewResponseController(w).Flush()
	}
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
	if failure != nil {
		cancel()
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
	result := <-done
	if result.panicked {
		return invocationResult{err: result.err, cleanup: result.err, aborted: true}
	}
	if errors.Is(result.err, context.Canceled) {
		return invocationResult{err: result.err, aborted: true}
	}
	if result.err != nil {
		result.completion = commandwire.Completion{ExitCode: 1, Error: &commandwire.CommandError{Code: "command_failed", Message: result.err.Error()}}
	}
	err := write(commandwire.Completed{Completion: result.completion})
	if result.completion.ExitCode != 0 && result.err == nil {
		result.err = fmt.Errorf("release exited with status %d", result.completion.ExitCode)
	}
	return invocationResult{err: errors.Join(result.err, err), cleanup: result.err, aborted: err != nil}
}
