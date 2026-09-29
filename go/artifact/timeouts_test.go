package artifact

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"net/url"
	"testing"
	"testing/synctest"
	"time"
)

// A roundTripFunc answers a request without a network.
type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(request *http.Request) (*http.Response, error) { return f(request) }

// A stalledBody yields the chunks it is given, each after the pause before it,
// then waits until the request ends.
type stalledBody struct {
	ctx    context.Context
	pauses []time.Duration
}

func (b *stalledBody) Read(p []byte) (int, error) {
	if len(b.pauses) > 0 {
		pause := b.pauses[0]
		b.pauses = b.pauses[1:]
		select {
		case <-time.After(pause):
			return copy(p, "x"), nil
		case <-b.ctx.Done():
			return 0, b.ctx.Err()
		}
	}
	<-b.ctx.Done()
	return 0, b.ctx.Err()
}

func (b *stalledBody) Close() error { return nil }

// A read that waits 60 s for bytes fails, and every byte that arrives starts the
// wait again.
func TestAReadOfABodyFailsAfter60SecondsWithoutBytes(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		client := &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
			return &http.Response{StatusCode: 200, ContentLength: -1, Body: &stalledBody{ctx: request.Context(), pauses: []time.Duration{59 * time.Second, 59 * time.Second}}}, nil
		})}
		start := time.Now()
		err := Download(t.Context(), client, "http://example.test/artifact", Digest{Size: 100}, &bytes.Buffer{})
		var failure *DownloadError
		if !errors.As(err, &failure) || failure.Message != "timed out waiting for the server" {
			t.Errorf("a stalled body: %v", err)
		}
		if elapsed := time.Since(start); elapsed != 59*time.Second+59*time.Second+60*time.Second {
			t.Errorf("the body was given up after %v, want 59 s and 59 s of bytes and then 60 s of none", elapsed)
		}
	})
}

// A connection that is not made in 15 s fails, and so does an answer that does
// not begin in 60 s.
func TestAConnectionThatIsNotMadeIn15SecondsFails(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		client := newClient(true, nil, func(ctx context.Context, network, address string) (net.Conn, error) {
			<-ctx.Done()
			return nil, ctx.Err()
		})
		start := time.Now()
		err := Download(t.Context(), client, "http://example.test/artifact", Digest{Size: 1}, io.Discard)
		var failure *DownloadError
		if !errors.As(err, &failure) {
			t.Errorf("a connection that is not made: %v", err)
		}
		if elapsed := time.Since(start); elapsed != 15*time.Second {
			t.Errorf("the connection was given up after %v, want 15 s", elapsed)
		}
	})
}

func TestAnAnswerThatDoesNotBeginIn60SecondsFails(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		server, client := net.Pipe()
		defer server.Close()
		// The server reads the request and never answers.
		go io.Copy(io.Discard, server)
		pipe := newClient(true, nil, func(context.Context, string, string) (net.Conn, error) { return client, nil })
		start := time.Now()
		err := Download(t.Context(), pipe, "http://example.test/artifact", Digest{Size: 1}, io.Discard)
		var failure *DownloadError
		if !errors.As(err, &failure) {
			t.Errorf("an answer that does not begin: %v", err)
		}
		if elapsed := time.Since(start); elapsed != 60*time.Second {
			t.Errorf("the answer was given up after %v, want 60 s", elapsed)
		}
	})
}

// The connection is ready within 15 s of the request's start, whatever it takes
// to make: a server that accepts the connection and never speaks TLS is given up
// at 15 s.
func TestATLSHandshakeThatNeverEndsFailsWithinTheConnectLimit(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		server, client := net.Pipe()
		defer server.Close()
		// The peer reads what it is sent and stays silent.
		go io.Copy(io.Discard, server)
		silent := newClient(false, nil, func(context.Context, string, string) (net.Conn, error) { return client, nil })
		start := time.Now()
		err := Download(t.Context(), silent, "https://example.test/artifact", Digest{Size: 1}, io.Discard)
		var failure *DownloadError
		if !errors.As(err, &failure) {
			t.Errorf("a handshake that never ends: %v", err)
		}
		if elapsed := time.Since(start); elapsed != 15*time.Second {
			t.Errorf("the handshake was given up after %v, want 15 s", elapsed)
		}
	})
}

// A slow dial and a slow handshake share the 15 s: 10 s of each is 5 s over.
func TestADialAndAHandshakeShareTheConnectBudget(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		server, client := net.Pipe()
		defer server.Close()
		go io.Copy(io.Discard, server)
		slow := newClient(false, nil, func(ctx context.Context, network, address string) (net.Conn, error) {
			select {
			case <-time.After(10 * time.Second):
				return client, nil
			case <-ctx.Done():
				return nil, ctx.Err()
			}
		})
		start := time.Now()
		err := Download(t.Context(), slow, "https://example.test/artifact", Digest{Size: 1}, io.Discard)
		var failure *DownloadError
		if !errors.As(err, &failure) || failure.Message != "timed out connecting to the server" {
			t.Errorf("a dial and a handshake over the budget: %v", err)
		}
		if elapsed := time.Since(start); elapsed != 15*time.Second {
			t.Errorf("the connection was given up after %v, want 15 s", elapsed)
		}
	})
}

// A proxy that accepts the connection and never answers CONNECT is given up at
// 15 s.
func TestAProxyThatNeverAnswersConnectFailsWithinTheConnectBudget(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		proxy, client := net.Pipe()
		defer proxy.Close()
		go io.Copy(io.Discard, proxy)
		viaProxy := newClient(false, func(*http.Request) (*url.URL, error) { return url.Parse("http://proxy.test:3128") },
			func(context.Context, string, string) (net.Conn, error) { return client, nil })
		start := time.Now()
		err := Download(t.Context(), viaProxy, "https://example.test/artifact", Digest{Size: 1}, io.Discard)
		var failure *DownloadError
		if !errors.As(err, &failure) || failure.Message != "timed out connecting to the server" {
			t.Errorf("a proxy that never answers: %v", err)
		}
		if elapsed := time.Since(start); elapsed != 15*time.Second {
			t.Errorf("the proxy was given up after %v, want 15 s", elapsed)
		}
	})
}
