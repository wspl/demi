// Package providertest supplies scripted vendors for provider boundary tests.
package providertest

import (
	"context"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
)

// Ending controls what happens after the scripted chunks.
type Ending uint8

const (
	Complete Ending = iota
	Open
	Silent
	Broken
)

type MockResponse struct {
	// Socket upgrades the response, reads one request message, then sends Chunks as text frames.
	Socket bool
	// Release holds the HTTP response open until the test permits completion.
	Release <-chan struct{}
	Status  int
	Headers http.Header
	Chunks  []string
	Ending  Ending
}
type RecordedRequest struct {
	Method      string
	URI         string
	Headers     http.Header
	Body        []byte
	SocketClose string
}

// MockVendor consumes route-specific responses before the general queue.
// Cleanup cancels every held response, drains handlers and closes the server.
type MockVendor struct {
	Server       *httptest.Server
	mu           sync.Mutex
	responses    []MockResponse
	routes       map[string][]MockResponse
	requests     []RecordedRequest
	changed      chan struct{}
	disconnected chan struct{}
	ctx          context.Context
	cancel       context.CancelFunc
}

func NewMockVendor(t *testing.T, responses ...MockResponse) *MockVendor {
	return newVendor(t, false, false, responses)
}
func NewTLSMockVendor(t *testing.T, responses ...MockResponse) *MockVendor {
	return newVendor(t, true, false, responses)
}
func newVendor(t *testing.T, tls, pipe bool, responses []MockResponse) *MockVendor {
	t.Helper()
	ctx, cancel := context.WithCancel(t.Context())
	v := &MockVendor{responses: responses, routes: make(map[string][]MockResponse), changed: make(chan struct{}), disconnected: make(chan struct{}, 1), ctx: ctx, cancel: cancel}
	v.Server = httptest.NewUnstartedServer(http.HandlerFunc(v.serve))
	var listener *pipeListener
	if pipe {
		v.Server.Listener.Close()
		listener = &pipeListener{pending: make(chan net.Conn), done: make(chan struct{})}
		v.Server.Listener = listener
	}
	if tls {
		v.Server.StartTLS()
	} else {
		v.Server.Start()
	}
	if pipe {
		v.Server.Client().Transport = &http.Transport{DialContext: listener.dial}
	}
	t.Cleanup(func() {
		v.cancel()
		v.Server.Client().CloseIdleConnections()
		v.Server.CloseClientConnections()
		v.Server.Close()
	})
	return v
}
func (v *MockVendor) Route(path string, responses ...MockResponse) {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.routes[path] = append(v.routes[path], responses...)
}
func (v *MockVendor) Requests() []RecordedRequest {
	v.mu.Lock()
	defer v.mu.Unlock()
	result := make([]RecordedRequest, len(v.requests))
	for i, r := range v.requests {
		r.Headers = r.Headers.Clone()
		r.Body = append([]byte(nil), r.Body...)
		result[i] = r
	}
	return result
}
func (v *MockVendor) WaitRequests(t *testing.T, count int) {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), 2*time.Minute)
	defer cancel()
	for {
		v.mu.Lock()
		n, changed := len(v.requests), v.changed
		v.mu.Unlock()
		if n >= count {
			return
		}
		select {
		case <-changed:
		case <-ctx.Done():
			t.Fatal("vendor requests did not arrive")
		}
	}
}
func (v *MockVendor) WaitDisconnect(t *testing.T) {
	t.Helper()
	ctx, cancel := context.WithTimeout(t.Context(), 2*time.Minute)
	defer cancel()
	select {
	case <-v.disconnected:
	case <-ctx.Done():
		t.Fatal("vendor client did not disconnect")
	}
}
func (v *MockVendor) serve(w http.ResponseWriter, r *http.Request) {
	body, err := io.ReadAll(r.Body)
	if err != nil {
		return
	}
	v.mu.Lock()
	requestIndex := len(v.requests)
	v.requests = append(v.requests, RecordedRequest{Method: r.Method, URI: r.URL.RequestURI(), Headers: r.Header.Clone(), Body: body})
	response := MockResponse{Status: 500, Chunks: []string{"MockVendor: no response scripted"}}
	if queue := v.routes[r.URL.Path]; len(queue) != 0 {
		response = queue[0]
		v.routes[r.URL.Path] = queue[1:]
	} else if len(v.responses) != 0 {
		response = v.responses[0]
		v.responses = v.responses[1:]
	}
	close(v.changed)
	v.changed = make(chan struct{})
	v.mu.Unlock()
	if response.Socket {
		v.serveSocket(w, r, response, requestIndex)
		return
	}
	if response.Ending != Silent {
		for name, values := range response.Headers {
			for _, value := range values {
				w.Header().Add(name, value)
			}
		}
		status := response.Status
		if status == 0 {
			status = 200
		}
		w.WriteHeader(status)
		controller := http.NewResponseController(w)
		if err := controller.Flush(); err != nil {
			return
		}
		for _, chunk := range response.Chunks {
			if _, err := io.WriteString(w, chunk); err != nil {
				return
			}
			if err := controller.Flush(); err != nil {
				return
			}
		}
	}
	if response.Release != nil {
		select {
		case <-response.Release:
		case <-r.Context().Done():
		case <-v.ctx.Done():
		}
	}
	switch response.Ending {
	case Open, Silent:
		select {
		case <-r.Context().Done():
		case <-v.ctx.Done():
		}
		select {
		case v.disconnected <- struct{}{}:
		default: /* Notifications coalesce when nobody waits. */
		}
	case Broken:
		panic(http.ErrAbortHandler) // net/http closes this response without its terminator.
	}
}

// NewPipeMockVendor runs the same HTTP scripts over net.Pipe, whose channel
// waits let testing/synctest advance vendor timeout tests without wall time.
func NewPipeMockVendor(t *testing.T, responses ...MockResponse) *MockVendor {
	return newVendor(t, false, true, responses)
}

type pipeListener struct {
	pending chan net.Conn
	done    chan struct{}
	once    sync.Once
}

func (l *pipeListener) Accept() (net.Conn, error) {
	select {
	case conn := <-l.pending:
		return conn, nil
	case <-l.done:
		return nil, net.ErrClosed
	}
}
func (l *pipeListener) Close() error { l.once.Do(func() { close(l.done) }); return nil }
func (*pipeListener) Addr() net.Addr { return &net.TCPAddr{IP: net.IPv4(127, 0, 0, 1), Port: 1} }
func (l *pipeListener) dial(ctx context.Context, _, _ string) (net.Conn, error) {
	client, server := net.Pipe()
	select {
	case l.pending <- server:
		return client, nil
	case <-ctx.Done():
		client.Close()
		server.Close()
		return nil, ctx.Err()
	case <-l.done:
		client.Close()
		server.Close()
		return nil, net.ErrClosed
	}
}

func (v *MockVendor) serveSocket(w http.ResponseWriter, r *http.Request, response MockResponse, index int) {
	for name, values := range response.Headers {
		for _, value := range values {
			w.Header().Add(name, value)
		}
	}
	conn, err := websocket.Accept(w, r, nil)
	if err != nil {
		return
	}
	defer conn.CloseNow()
	ctx, cancel := context.WithCancel(r.Context())
	stop := context.AfterFunc(v.ctx, cancel)
	defer func() { stop(); cancel() }()
	_, request, err := conn.Read(ctx)
	if err != nil {
		return
	}
	v.mu.Lock()
	v.requests[index].Body = append([]byte(nil), request...)
	v.mu.Unlock()
	for _, chunk := range response.Chunks {
		if err := conn.Write(ctx, websocket.MessageText, []byte(chunk)); err != nil {
			return
		}
	}
	if response.Ending == Complete {
		conn.Close(websocket.StatusNormalClosure, "script ended")
		return
	}
	if response.Ending == Broken {
		return
	}
	_, _, err = conn.Read(ctx)
	v.mu.Lock()
	if err != nil {
		v.requests[index].SocketClose = err.Error()
	}
	v.mu.Unlock()
	select {
	case v.disconnected <- struct{}{}:
	default:
	}
}
