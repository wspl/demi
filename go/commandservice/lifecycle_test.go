package commandservice_test

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandservice/servicetest"
)

// closing is a handler that records when its calls return and when it closes.
type closing struct {
	operations
	order events
}

func (c *closing) Close(context.Context) error {
	c.order.add("close")
	return nil
}

func newClosing(started signal) *closing {
	c := &closing{}
	c.operations = operations{
		"hold": func(call *commandservice.Call) (commandservice.Completion, error) {
			started.send()
			<-call.Context().Done()
			c.order.add("call returned")
			return commandservice.Completion{}, call.Context().Err()
		},
	}
	return c
}

func TestServeEndsAfterItsCallsWhenThePeerClosesTheConnection(t *testing.T) {
	started := newSignal()
	handler := newClosing(started)
	server := servicetest.Start(t, handler)
	if _, err := server.Client.Invoke(testContext(t), invocation("hold")); err != nil {
		t.Fatal(err)
	}
	started.receive(t)
	if err := server.Client.Close(); err != nil {
		t.Fatal(err)
	}
	if err := server.Wait(testContext(t)); err != nil {
		t.Errorf("Serve returned %v after the peer closed the connection", err)
	}
	if got := handler.order.all(); len(got) != 2 || got[0] != "call returned" || got[1] != "close" {
		t.Errorf("order = %v, want every call to return before the handler closes", got)
	}
}

func TestServeEndsAfterItsCallsWhenItsContextEnds(t *testing.T) {
	started := newSignal()
	handler := newClosing(started)
	server := servicetest.Start(t, handler)
	stream, err := server.Client.Invoke(testContext(t), invocation("hold"))
	if err != nil {
		t.Fatal(err)
	}
	started.receive(t)
	server.Stop()
	if err := server.Wait(testContext(t)); err != nil {
		t.Errorf("Serve returned %v after its context ended", err)
	}
	if got := handler.order.all(); len(got) != 2 || got[0] != "call returned" || got[1] != "close" {
		t.Errorf("order = %v, want every call to return before the handler closes", got)
	}
	// The stream that was running ends without a completion, which no
	// handler that was cancelled sends.
	if err := drain(stream); err == nil {
		t.Error("a cancelled call ended with a completion")
	}
}

// requireClosed fails the test unless the other end of conn was closed.
func requireClosed(t *testing.T, conn net.Conn) {
	t.Helper()
	// A connection that stayed open would block the read; the deadline guards
	// against that hang. A pipe whose other end closed refuses a deadline, and
	// its read does not block.
	if err := conn.SetReadDeadline(time.Now().Add(hang)); err != nil && !errors.Is(err, io.ErrClosedPipe) {
		t.Fatal(err)
	}
	if _, err := conn.Read(make([]byte, 1)); err != io.EOF {
		t.Errorf("reading the connection: %v, want io.EOF", err)
	}
}

// A service whose handshake did not complete has served nothing, so it has
// nothing to release: it ends without calling its handler's Close, closes the
// connection it was given, and returns ErrCancelled when its context cut the
// handshake short.
func TestAServiceCutShortBeforeItsHandshakeClosesTheConnectionAndNotItsHandler(t *testing.T) {
	clientEnd, serviceEnd := net.Pipe()
	handler := newClosing(newSignal())
	ctx, stop := context.WithCancel(testContext(t))
	served := make(chan error, 1)
	go func() { served <- commandservice.Serve(ctx, serviceEnd, handler) }()
	stop()
	if err := <-served; !errors.Is(err, commandservice.ErrCancelled) {
		t.Errorf("Serve returned %v after its context ended, want ErrCancelled", err)
	}
	requireClosed(t, clientEnd)
	if got := handler.order.all(); len(got) != 0 {
		t.Errorf("the handler saw %v of a service that served nothing", got)
	}
}

// A peer that closes its side before the handshake is complete has failed it.
func TestAPeerThatClosesBeforeTheHandshakeFailsTheServiceWithoutClosingItsHandler(t *testing.T) {
	clientEnd, serviceEnd := net.Pipe()
	handler := newClosing(newSignal())
	served := make(chan error, 1)
	go func() { served <- commandservice.Serve(testContext(t), serviceEnd, handler) }()
	if err := clientEnd.Close(); err != nil {
		t.Fatal(err)
	}
	if err := <-served; err == nil || errors.Is(err, commandservice.ErrCancelled) {
		t.Errorf("Serve returned %v after the peer closed before the handshake, want its failure", err)
	}
	if got := handler.order.all(); len(got) != 0 {
		t.Errorf("the handler saw %v of a service that served nothing", got)
	}
}

func TestShutdownLetsRunningCallsFinishThenEndsTheService(t *testing.T) {
	server := servicetest.Start(t, operations{"hold": hold})
	client := server.Client
	held, err := client.Invoke(testContext(t), invocation("hold"))
	if err != nil {
		t.Fatal(err)
	}
	if err := client.Shutdown(testContext(t)); err != nil {
		t.Fatal(err)
	}
	// Admission stopped: no new call starts.
	if _, err := client.Invoke(testContext(t), invocation("hold")); err == nil {
		t.Error("a call was admitted after the shutdown")
	}
	// The call that was running still ends normally, with its completion.
	completed := false
	if _, err := held.Next(); err != nil {
		t.Fatal(err)
	}
	if err := held.End(); err != nil {
		t.Fatal(err)
	}
	for {
		record, err := held.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			t.Fatal(err)
		}
		completed = completed || record.Kind == commandservice.RecordCompletion
	}
	if !completed {
		t.Error("the call that was running when the service drained ended without its completion")
	}
	if err := server.Wait(testContext(t)); err != nil {
		t.Errorf("Serve returned %v after a drain", err)
	}
}

func TestAnInvalidCatalogFailsServeBeforeItServes(t *testing.T) {
	for name, handler := range map[string]commandservice.Handler{
		"none":     operations{},
		"repeated": repeating{},
		"unnamed":  operations{"": short},
	} {
		clientEnd, serviceEnd := net.Pipe()
		err := commandservice.Serve(testContext(t), serviceEnd, handler)
		var invalid *commandservice.InvalidError
		if !errors.As(err, &invalid) {
			t.Errorf("%s: Serve returned %v, want an InvalidError", name, err)
		}
		// Serve closed the connection it was given.
		requireClosed(t, clientEnd)
	}
}

// repeating is a handler that names the same operation twice.
type repeating struct{ operations }

func (repeating) Operations() []string { return []string{"same", "same"} }

// riggedConn is the service's end of a connection whose writes a test can
// make fail, as a peer's closed pipe does.
type riggedConn struct {
	net.Conn
	failWrites atomic.Bool
	failure    error
}

func (c *riggedConn) Write(p []byte) (int, error) {
	if c.failWrites.Load() {
		return 0, c.failure
	}
	return c.Conn.Write(p)
}

// serveRigged serves a handler with one call that holds until its input ends,
// over a connection whose writes fail with failure once told to, and starts
// that call.
func serveRigged(t *testing.T, failure error) (conn *riggedConn, held *commandservice.Stream, client *commandservice.Client, served <-chan error) {
	t.Helper()
	clientEnd, serviceEnd := net.Pipe()
	conn = &riggedConn{Conn: serviceEnd, failure: failure}
	started := newSignal()
	handler := operations{"hold": func(call *commandservice.Call) (commandservice.Completion, error) {
		started.send()
		return hold(call)
	}}
	result := make(chan error, 1)
	go func() { result <- commandservice.Serve(context.Background(), conn, handler) }()
	client, err := commandservice.Connect(testContext(t), clientEnd)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { client.Close() })
	held, err = client.Invoke(testContext(t), invocation("hold"))
	if err != nil {
		t.Fatal(err)
	}
	started.receive(t)
	return conn, held, client, result
}

// A peer that closes its transport while the service still has frames to write
// ends the connection; after an answered shutdown that is no failure, and
// before one it is. What the platform says of a write into a closed pipe is
// closedPipeErrors; a pipe made in memory and a stream that ends in the middle
// of a frame say it the same way on every platform. Each case runs in a fake
// clock, so the service's waits (the one second after its GOAWAY, the five of
// the cancellation grace) cost no time, and a call that the service waited for
// in vain would show as a Serve that never returns, not as a slow test.
func TestAPeerThatClosesItsTransportIsNoFailureOnlyAfterAnAnsweredShutdown(t *testing.T) {
	for _, closed := range append([]error{io.ErrClosedPipe, io.ErrUnexpectedEOF}, closedPipeErrors...) {
		for _, shutdown := range []bool{true, false} {
			synctest.Test(t, func(t *testing.T) {
				conn, held, client, served := serveRigged(t, closed)
				if shutdown {
					if err := client.Shutdown(testContext(t)); err != nil {
						t.Fatal(err)
					}
				}
				// The held call completes into a closed pipe.
				conn.failWrites.Store(true)
				_ = held.End()
				err := <-served
				if shutdown && err != nil {
					t.Errorf("%v: after an answered shutdown, Serve returned %v", closed, err)
				}
				if !shutdown && !errors.Is(err, closed) {
					t.Errorf("%v: without a shutdown, Serve returned %v, want the failure of the pipe", closed, err)
				}
			})
		}
	}
}

// What the service's own close does to a read is nothing the peer said: the
// connection tells the peer's end from its own close by whether the service had
// closed it. The test reads and closes in both orders, and needs no scheduling:
// either way the read fails after the close.
func TestAReadThatFailsBecauseTheServiceClosedTheConnectionIsNotThePeersEnd(t *testing.T) {
	for _, closeFirst := range []bool{false, true} {
		peer, serviceEnd := net.Pipe()
		conn := commandservice.NewServiceConn(serviceEnd)
		read := make(chan error, 1)
		startRead := func() {
			go func() {
				_, err := conn.Read(make([]byte, 1))
				read <- err
			}()
		}
		if !closeFirst {
			startRead()
		}
		if err := conn.Close(); err != nil {
			t.Fatal(err)
		}
		if closeFirst {
			startRead()
		}
		if err := <-read; err == nil {
			t.Fatalf("closed first %v: the read of a closed connection succeeded", closeFirst)
		}
		if conn.PeerEnded() {
			t.Errorf("closed first %v: the service's own close was taken for the peer's end", closeFirst)
		}
		if err := conn.Failure(); err != nil {
			t.Errorf("closed first %v: the service's own close was taken for the peer's failure: %v", closeFirst, err)
		}
		peer.Close()
	}
}

// The peer that closes its side ends the connection's input, which is all the
// service learns of it.
func TestAReadThatEndsBecauseThePeerClosedIsThePeersEnd(t *testing.T) {
	peer, serviceEnd := net.Pipe()
	conn := commandservice.NewServiceConn(serviceEnd)
	t.Cleanup(func() { conn.Close() })
	if err := peer.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := conn.Read(make([]byte, 1)); err != io.EOF {
		t.Fatalf("the read after the peer closed returned %v, want the end of input", err)
	}
	if !conn.PeerEnded() {
		t.Error("the end of the peer's input was not taken for the peer's end")
	}
	if err := conn.Failure(); err != nil {
		t.Errorf("the end of the peer's input was taken for a failure: %v", err)
	}
}

// brokenResponse is the response of a caller that is gone: its headers go out,
// and nothing after them is written.
type brokenResponse struct{ header http.Header }

func (b *brokenResponse) Header() http.Header {
	if b.header == nil {
		b.header = http.Header{}
	}
	return b.header
}
func (b *brokenResponse) WriteHeader(int)           {}
func (b *brokenResponse) Flush()                    {}
func (b *brokenResponse) Write([]byte) (int, error) { return 0, io.ErrClosedPipe }

// A handler that returns while the caller has gone is joined at once: its
// result was taken by the service, so there is nothing to wait for, and waiting
// out the cancellation grace would retire a service whose handler did nothing
// wrong. The service takes the result or the last records first, whichever it
// meets, so the call is made many times.
func TestAHandlerThatReturnedIsNotWaitedForWhenItsAnswerCannotBeWritten(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		service, fault, err := commandservice.ServiceHandler(operations{"write": func(call *commandservice.Call) (commandservice.Completion, error) {
			_, err := call.Stdout.Write([]byte("output"))
			return commandservice.Completion{}, err
		}})
		if err != nil {
			t.Fatal(err)
		}
		body := metadataBodyFor(t, invocation("write"))
		start := time.Now()
		for range 40 {
			service.ServeHTTP(&brokenResponse{}, httptest.NewRequest(http.MethodPost, commandservice.InvokePath, bytes.NewReader(body)))
		}
		if elapsed := time.Since(start); elapsed != 0 {
			t.Errorf("the service waited %v for handlers that had returned", elapsed)
		}
		if err := fault(); err != nil {
			t.Errorf("the service faulted: %v", err)
		}
	})
}
