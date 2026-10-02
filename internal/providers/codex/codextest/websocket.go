package codextest

import (
	"context"
	"errors"
	"io"
	"net"
	"net/http"
	"net/http/httputil"
	"net/url"
	"sync"
	"testing"

	"github.com/coder/websocket"
)

// StepKind selects a scripted action after the client's first message.
type StepKind uint8

const (
	// Send writes the step's text as a vendor event.
	Send StepKind = iota
	// Close sends a normal close frame.
	Close
	// Drop ends the connection without a close frame.
	Drop
)

// Step is one scripted action after the client's first message.
type Step struct {
	Kind StepKind
	Text string
}

// Handshake selects how a scripted connection opens.
type Handshake uint8

const (
	// Accept upgrades the connection and plays its steps.
	Accept Handshake = iota
	// Reject responds with the script's HTTP status.
	Reject
	// Disconnect drops the TCP connection before its handshake.
	Disconnect
)

// Script describes one handshake and its subsequent messages.
type Script struct {
	Handshake Handshake
	Status    int
	Steps     []Step
}

// Connection records one handshake, its request messages and the client's close.
type Connection struct {
	Headers     http.Header
	Received    []string
	CloseReason *string
}

// FakeWebSocket serves scripted WebSockets over in-memory connections. Non-upgrade
// requests are forwarded to a scripted vendor when a fallback URL is supplied.
// Its client is required; no network request reaches codex.test.
type FakeWebSocket struct {
	mu             sync.Mutex // Protects scripts, recordings, sockets and change notifications.
	closed         bool
	scripts        []Script
	connections    []Connection
	sockets        map[*websocket.Conn]bool
	changed        chan struct{}
	server         *http.Server
	listener       *pipeListener
	client         *http.Client
	transport      *http.Transport
	proxyTransport *http.Transport
	cancel         context.CancelFunc
	handlers       sync.WaitGroup
	served         chan struct{}
	once           sync.Once
}

// Start starts the scripted backend and registers cancellation and joins with t.
func Start(t testing.TB, scripts []Script, fallback string) *FakeWebSocket {
	t.Helper()
	ctx, cancel := context.WithCancel(context.Background())
	f := &FakeWebSocket{scripts: scripts, sockets: make(map[*websocket.Conn]bool), changed: make(chan struct{}), listener: &pipeListener{accepted: make(chan net.Conn), done: make(chan struct{})}, cancel: cancel, served: make(chan struct{})}
	f.transport = &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		client, server := net.Pipe()
		select {
		case f.listener.accepted <- server:
			return client, nil
		case <-ctx.Done():
			_ = client.Close()
			_ = server.Close()
			return nil, ctx.Err()
		case <-f.listener.done:
			_ = client.Close()
			_ = server.Close()
			return nil, net.ErrClosed
		}
	}}
	f.client = &http.Client{Transport: f.transport}
	var proxy *httputil.ReverseProxy
	if fallback != "" {
		target, err := url.Parse(fallback)
		if err != nil {
			t.Fatal(err)
		}
		proxy = httputil.NewSingleHostReverseProxy(target)
		f.proxyTransport = &http.Transport{}
		proxy.Transport = f.proxyTransport
	}
	f.server = &http.Server{BaseContext: func(net.Listener) context.Context { return ctx }, Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		f.mu.Lock()
		if f.closed {
			f.mu.Unlock()
			return
		}
		f.handlers.Add(1)
		f.mu.Unlock()
		defer f.handlers.Done()
		if r.Header.Get("Upgrade") != "websocket" {
			if proxy != nil {
				proxy.ServeHTTP(w, r)
			} else {
				w.WriteHeader(502)
			}
			return
		}
		f.serve(ctx, w, r)
	})}
	go func() {
		defer close(f.served)
		_ = f.server.Serve(f.listener)
	}() // Shutdown closes the listener and joins Serve.
	t.Cleanup(f.Close)
	return f
}

// BackendURL returns the in-memory backend's base URL.
func (*FakeWebSocket) BackendURL() string { return "http://codex.test/backend-api" }

// Client returns the HTTP client connected to this scripted backend.
func (f *FakeWebSocket) Client() *http.Client { return f.client }

// Connections returns independent copies of the recordings.
func (f *FakeWebSocket) Connections() []Connection {
	f.mu.Lock()
	defer f.mu.Unlock()
	result := make([]Connection, len(f.connections))
	for i, c := range f.connections {
		result[i] = Connection{Headers: c.Headers.Clone(), Received: append([]string(nil), c.Received...)}
		if c.CloseReason != nil {
			reason := *c.CloseReason
			result[i].CloseReason = &reason
		}
	}
	return result
}

// CloseReason waits for the client's close frame or cancellation.
func (f *FakeWebSocket) CloseReason(ctx context.Context, index int) (string, error) {
	for {
		f.mu.Lock()
		if index < len(f.connections) && f.connections[index].CloseReason != nil {
			reason := *f.connections[index].CloseReason
			f.mu.Unlock()
			return reason, nil
		}
		changed := f.changed
		f.mu.Unlock()
		select {
		case <-changed:
		case <-ctx.Done():
			return "", ctx.Err()
		}
	}
}

// Close releases every connection and joins all server handlers.
func (f *FakeWebSocket) Close() {
	f.once.Do(func() {
		f.mu.Lock()
		f.closed = true
		f.mu.Unlock()
		f.cancel()
		_ = f.server.Close() // Cleanup closes the listener and ordinary HTTP connections.
		<-f.served
		f.mu.Lock()
		sockets := make([]*websocket.Conn, 0, len(f.sockets))
		for socket := range f.sockets {
			sockets = append(sockets, socket)
		}
		f.mu.Unlock()
		for _, socket := range sockets {
			_ = socket.CloseNow()
		} // Hijacked connections belong to the fixture.
		f.handlers.Wait()
		f.transport.CloseIdleConnections()
		if f.proxyTransport != nil {
			f.proxyTransport.CloseIdleConnections()
		}
	})
}
func (f *FakeWebSocket) serve(ctx context.Context, w http.ResponseWriter, r *http.Request) {
	f.mu.Lock()
	script := Script{Handshake: Disconnect}
	if len(f.scripts) > 0 {
		script = f.scripts[0]
		f.scripts = f.scripts[1:]
	}
	index := len(f.connections)
	f.connections = append(f.connections, Connection{Headers: r.Header.Clone()})
	f.mu.Unlock()
	if script.Handshake == Disconnect {
		conn, _, err := http.NewResponseController(w).Hijack()
		if err == nil {
			_ = conn.Close()
		} // Scripted abrupt close.
		return
	}
	if script.Handshake == Reject {
		w.WriteHeader(script.Status)
		_, _ = io.WriteString(w, "refused")
		return
	}
	w.Header().Set("X-Codex-Primary-Used-Percent", "12")
	socket, err := websocket.Accept(w, r, nil)
	if err != nil {
		return
	}
	socket.SetReadLimit(64 * 1024 * 1024)
	f.mu.Lock()
	if f.closed {
		f.mu.Unlock()
		_ = socket.CloseNow() // Shutdown overtook the handshake.
		return
	}
	f.sockets[socket] = true
	f.mu.Unlock()
	defer func() {
		_ = socket.CloseNow() // Release the hijacked connection on every script outcome.
		f.mu.Lock()
		delete(f.sockets, socket)
		f.mu.Unlock()
	}()
	_, first, err := socket.Read(ctx)
	if err != nil {
		return
	}
	f.mu.Lock()
	f.connections[index].Received = append(f.connections[index].Received, string(first))
	f.mu.Unlock()
	for _, step := range script.Steps {
		if step.Kind == Drop {
			return
		}
		if step.Kind == Close {
			_ = socket.Close(websocket.StatusNormalClosure, "")
			return
		}
		if err := socket.Write(ctx, websocket.MessageText, []byte(step.Text)); err != nil {
			return
		}
	}
	for {
		_, data, err := socket.Read(ctx)
		if err != nil {
			var closeError websocket.CloseError
			if errors.As(err, &closeError) {
				f.mu.Lock()
				f.connections[index].CloseReason = &closeError.Reason
				close(f.changed)
				f.changed = make(chan struct{})
				f.mu.Unlock()
			}
			return
		}
		f.mu.Lock()
		f.connections[index].Received = append(f.connections[index].Received, string(data))
		f.mu.Unlock()
	}
}

type pipeListener struct {
	accepted chan net.Conn
	done     chan struct{}
	once     sync.Once
}

func (l *pipeListener) Accept() (net.Conn, error) {
	select {
	case conn := <-l.accepted:
		return conn, nil
	case <-l.done:
		return nil, net.ErrClosed
	}
}
func (l *pipeListener) Close() error { l.once.Do(func() { close(l.done) }); return nil }
func (*pipeListener) Addr() net.Addr { return &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1)} }
