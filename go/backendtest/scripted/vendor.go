package scripted

import (
	"bytes"
	"context"
	"encoding/json/v2"
	"io"
	"net"
	"net/http"
	"sync"
	"testing"
	"time"
)

// vendorPatience is how long Received and Disconnected wait.
const vendorPatience = 5 * time.Second

// openPing is how often an open response sends a comment, so that the server
// notices its client leaving.
const openPing = 10 * time.Millisecond

// brokenPause lets the server flush what came before the connection breaks.
const brokenPause = 20 * time.Millisecond

type ending int

const (
	// The zero ending ends the body after its chunks.
	_ ending = iota
	// endOpen keeps the body open until the client leaves.
	endOpen
	// endSilent never answers: no status, no headers.
	endSilent
	// endBroken breaks the connection before the body ends.
	endBroken
)

// A Gate holds responses until a scenario lets them through: each permit lets
// one response go, and a response whose client leaves first ends without one.
type Gate struct {
	permits chan struct{}
}

// NewGate returns a gate that holds every response that waits at it.
func NewGate() *Gate {
	return &Gate{permits: make(chan struct{}, 1024)}
}

// Permit lets count responses go, the ones that wait and the ones that come.
func (g *Gate) Permit(count int) {
	for range count {
		g.permits <- struct{}{}
	}
}

// Available is how many permits no response has taken.
func (g *Gate) Available() int {
	return len(g.permits)
}

// A Response is a scripted answer. The builder methods return a changed copy.
type Response struct {
	status  int
	headers [][2]string
	chunks  [][]byte
	ending  ending
	gate    *Gate
}

// Status is an empty answer with status.
func Status(status int) *Response {
	return &Response{status: status}
}

// EventStream is a 200 event stream whose body is text, sent in one chunk.
func EventStream(text string) *Response {
	return Status(200).Header("content-type", "text/event-stream").Chunk([]byte(text))
}

// Silent is a request that is never answered: the server reads it and sends
// nothing, not even a status.
func Silent() *Response {
	return &Response{status: 200, ending: endSilent}
}

func (r *Response) clone() *Response {
	copied := *r
	copied.headers = append([][2]string(nil), r.headers...)
	copied.chunks = append([][]byte(nil), r.chunks...)
	return &copied
}

// Header adds a header; a repeated name sends the header again.
func (r *Response) Header(name, value string) *Response {
	next := r.clone()
	next.headers = append(next.headers, [2]string{name, value})
	return next
}

// Chunk adds a body chunk, sent as one write.
func (r *Response) Chunk(chunk []byte) *Response {
	next := r.clone()
	next.chunks = append(next.chunks, chunk)
	return next
}

// After holds the response, status and all, until the gate lets it go; a client
// that leaves meanwhile ends it.
func (r *Response) After(gate *Gate) *Response {
	next := r.clone()
	next.gate = gate
	return next
}

// StayOpen keeps the response open after its chunks until the client leaves.
// The server sends an event-stream comment every 10 ms meanwhile, so that it
// notices the client leaving.
func (r *Response) StayOpen() *Response {
	next := r.clone()
	next.ending = endOpen
	return next
}

// BreakOff breaks the connection after the chunks, before the body is
// complete.
func (r *Response) BreakOff() *Response {
	next := r.clone()
	next.ending = endBroken
	return next
}

// A Request is a request a vendor received.
type Request struct {
	Method string
	// Path is the request's path, without its query; URI is with it.
	Path   string
	URI    string
	Header http.Header
	Body   []byte
}

// JSON decodes the body as JSON, or fails the test.
func (r Request) JSON(t testing.TB) any {
	t.Helper()
	var value any
	if err := json.Unmarshal(r.Body, &value); err != nil {
		t.Fatalf("the request body is not JSON: %v\n%s", err, r.Body)
	}
	return value
}

// A Vendor is an HTTP server that answers each request with the next scripted
// response and records every request. It stops with the test.
type Vendor struct {
	address string
	server  *http.Server

	mu           sync.Mutex
	handler      func(Request) *Response
	responses    []*Response
	routes       map[string][]*Response
	requests     []Request
	requested    chan struct{}
	disconnected chan struct{}
}

// StartVendor starts a vendor on a free loopback port.
func StartVendor(t testing.TB) *Vendor {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	v := &Vendor{
		address:      listener.Addr().String(),
		routes:       map[string][]*Response{},
		requested:    make(chan struct{}),
		disconnected: make(chan struct{}, 64),
	}
	v.server = &http.Server{Handler: http.HandlerFunc(v.answer), ReadHeaderTimeout: 10 * time.Second}
	served := make(chan struct{})
	go func() {
		defer close(served)
		// Serve returns http.ErrServerClosed when the test ends.
		_ = v.server.Serve(listener)
	}()
	t.Cleanup(func() {
		// Open responses end with their connections.
		_ = v.server.Close()
		<-served
	})
	return v
}

// URL is the URL of path on this server, such as /v1.
func (v *Vendor) URL(path string) string {
	return "http://" + v.address + path
}

// Respond queues the answer to the next request.
func (v *Vendor) Respond(response *Response) {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.responses = append(v.responses, response)
}

// RespondAt queues the answer to the next request to path, such as
// /oauth/token, which it takes before the answers of Respond: concurrent
// requests to different paths then get their own answers whatever order they
// arrive in.
func (v *Vendor) RespondAt(path string, response *Response) {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.routes[path] = append(v.routes[path], response)
}

// Handle sets a function that answers requests by what they carry, for a
// scenario whose requests arrive in an order it cannot script, such as those of
// a conversation and its children. It answers before the queues do; a request
// it answers nil for takes the next queued response. It runs once for each
// request, in the order they arrive, and must not call the vendor.
func (v *Vendor) Handle(handler func(Request) *Response) {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.handler = handler
}

// Requests returns every request received so far, in order.
func (v *Vendor) Requests() []Request {
	v.mu.Lock()
	defer v.mu.Unlock()
	return append([]Request(nil), v.requests...)
}

// Received waits until the vendor has received count requests, at most five
// seconds.
func (v *Vendor) Received(t testing.TB, count int) {
	t.Helper()
	timeout := time.After(vendorPatience)
	for {
		v.mu.Lock()
		received := len(v.requests)
		next := v.requested
		v.mu.Unlock()
		if received >= count {
			return
		}
		select {
		case <-next:
		case <-timeout:
			t.Fatalf("the vendor received %d requests, not %d", received, count)
		}
	}
}

// Disconnected waits until a client leaves a response that was still open, at
// most five seconds.
func (v *Vendor) Disconnected(t testing.TB) {
	t.Helper()
	select {
	case <-v.disconnected:
	case <-time.After(vendorPatience):
		t.Fatal("the client did not leave the open response")
	}
}

func (v *Vendor) answer(w http.ResponseWriter, r *http.Request) {
	body, err := io.ReadAll(r.Body)
	if err != nil {
		return
	}
	received := Request{
		Method: r.Method,
		Path:   r.URL.Path,
		URI:    r.URL.RequestURI(),
		Header: r.Header.Clone(),
		Body:   bytes.Clone(body),
	}
	v.mu.Lock()
	var scripted *Response
	if v.handler != nil {
		scripted = v.handler(received)
	}
	if routed := v.routes[r.URL.Path]; scripted == nil && len(routed) > 0 {
		scripted = routed[0]
		v.routes[r.URL.Path] = routed[1:]
	}
	v.requests = append(v.requests, received)
	close(v.requested)
	v.requested = make(chan struct{})
	if scripted == nil && len(v.responses) > 0 {
		scripted = v.responses[0]
		v.responses = v.responses[1:]
	}
	v.mu.Unlock()
	if scripted == nil {
		http.Error(w, "the vendor has no response scripted", http.StatusInternalServerError)
		return
	}
	if scripted.ending == endSilent {
		// The handler never returns, so the server sends nothing; the
		// connection closes when the client leaves or the vendor stops.
		<-r.Context().Done()
		return
	}
	if scripted.gate != nil {
		select {
		case <-scripted.gate.permits:
		case <-r.Context().Done():
			return
		}
	}
	for _, header := range scripted.headers {
		w.Header().Add(header[0], header[1])
	}
	w.WriteHeader(scripted.status)
	flusher := http.NewResponseController(w)
	// A client that left makes the flushes fail, and the handler ends with its
	// request's context.
	_ = flusher.Flush()
	for _, chunk := range scripted.chunks {
		if _, err := w.Write(chunk); err != nil {
			return
		}
		_ = flusher.Flush()
	}
	switch scripted.ending {
	case endOpen:
		v.holdOpen(w, r.Context(), flusher)
	case endBroken:
		time.Sleep(brokenPause)
		// The server drops the connection without ending the body.
		panic(http.ErrAbortHandler)
	}
}

// holdOpen keeps the response open with a comment every 10 ms until the client
// leaves, and tells Disconnected.
func (v *Vendor) holdOpen(w http.ResponseWriter, ctx context.Context, flusher *http.ResponseController) {
	defer func() {
		select {
		case v.disconnected <- struct{}{}:
		default:
		}
	}()
	ticker := time.NewTicker(openPing)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			if _, err := w.Write([]byte(": ping\n\n")); err != nil {
				return
			}
			if err := flusher.Flush(); err != nil {
				return
			}
		}
	}
}
