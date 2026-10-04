package providertest

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"sync"
	"testing"
	"time"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

// Ending controls what a scripted response does after sending its chunks.
type Ending uint8

// Ways a scripted vendor response can end.
const (
	// Complete identifies a scripted response that ends normally.
	Complete Ending = iota
	// Open identifies a scripted response kept open until cancellation.
	Open
	// Silent identifies a scripted response that sends no headers.
	Silent
	// Broken identifies a scripted response whose connection is broken.
	Broken
)

// MockResponse is one scripted vendor answer. Headers preserve repeated values.
type MockResponse struct {
	Status  int
	Headers http.Header
	Chunks  [][]byte
	Ending  Ending
}

// EventStream returns a completed event stream response.
func EventStream(text string) MockResponse {
	return MockResponse{
		Status:  200,
		Headers: http.Header{"Content-Type": []string{"text/event-stream"}},
		Chunks:  [][]byte{[]byte(text)},
	}
}

// RecordedRequest is the method, path/query, headers and body the vendor received.
type RecordedRequest struct {
	Method  string
	URI     string
	Headers http.Header
	Body    []byte
}

// JSON reads the request body as JSON or fails the test.
func (r RecordedRequest) JSON(t testing.TB) any {
	t.Helper()
	var value any
	if err := json.Unmarshal(r.Body, &value); err != nil {
		t.Fatal(err)
	}
	return value
}

// Header reads the first value of a request header.
func (r RecordedRequest) Header(name string) string { return r.Headers.Get(name) }

// MockVendor serves queued answers on loopback and records requests.
type MockVendor struct {
	t            testing.TB
	server       *httptest.Server
	mu           sync.Mutex
	responses    []MockResponse
	routes       map[string][]MockResponse
	requests     []RecordedRequest
	changed      chan struct{}
	disconnected chan struct{}
	cancel       context.CancelFunc
	once         sync.Once
}

// StartVendor starts an HTTP server and registers cleanup with the test.
func StartVendor(t testing.TB) *MockVendor { return startVendor(t, nil) }

// StartTLSVendor starts HTTPS using the supplied certificate chain and private key.
func StartTLSVendor(t testing.TB, certificate, key []byte) *MockVendor {
	t.Helper()
	pair, err := tls.X509KeyPair(certificate, key)
	if err != nil {
		t.Fatal(err)
	}
	return startVendor(t, &pair)
}

func startVendor(t testing.TB, certificate *tls.Certificate) *MockVendor {
	ctx, cancel := context.WithCancel(context.Background())
	vendor := &MockVendor{
		t:            t,
		routes:       make(map[string][]MockResponse),
		changed:      make(chan struct{}),
		disconnected: make(chan struct{}, 1),
		cancel:       cancel,
	}
	server := httptest.NewUnstartedServer(
		http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { vendor.answer(ctx, w, r) }),
	)
	vendor.server = server
	if certificate == nil {
		server.Start()
	} else {
		server.TLS = &tls.Config{Certificates: []tls.Certificate{*certificate}, MinVersion: tls.VersionTLS12}
		server.StartTLS()
	}
	t.Cleanup(vendor.Close)
	return vendor
}

// Close cancels open responses, stops accepting and joins the server's handlers.
func (v *MockVendor) Close() {
	v.once.Do(func() {
		v.cancel()
		v.server.CloseClientConnections()
		v.server.Close()
	})
}

// URL returns the URL for path, including any query.
func (v *MockVendor) URL(path string) string { return v.server.URL + path }

// Client returns a client trusting this server's test certificate.
func (v *MockVendor) Client() *http.Client { return v.server.Client() }

// Respond queues the next default answer.
func (v *MockVendor) Respond(response MockResponse) {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.responses = append(v.responses, response)
}

// RespondAt queues an answer for one path before default answers are considered.
func (v *MockVendor) RespondAt(path string, response MockResponse) {
	v.mu.Lock()
	defer v.mu.Unlock()
	v.routes[path] = append(v.routes[path], response)
}

// Requests returns all received requests in arrival order.
func (v *MockVendor) Requests() []RecordedRequest {
	v.mu.Lock()
	defer v.mu.Unlock()
	out := make([]RecordedRequest, len(v.requests))
	for i, r := range v.requests {
		out[i] = r
		out[i].Headers = r.Headers.Clone()
		out[i].Body = append([]byte{}, r.Body...)
	}
	return out
}

// Received waits for count requests with a five-second hang guard.
func (v *MockVendor) Received(ctx context.Context, count int) {
	v.t.Helper()
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	for {
		v.mu.Lock()
		received, changed := len(v.requests), v.changed
		v.mu.Unlock()
		if received >= count {
			return
		}
		select {
		case <-changed:
		case <-ctx.Done():
			v.t.Fatalf("vendor did not receive %d requests: %v", count, ctx.Err())
		}
	}
}

// Disconnected waits for a client to leave an open response.
func (v *MockVendor) Disconnected(ctx context.Context) {
	v.t.Helper()
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	select {
	case <-v.disconnected:
	case <-ctx.Done():
		v.t.Fatalf("client did not leave open response: %v", ctx.Err())
	}
}

func (v *MockVendor) answer(ctx context.Context, w http.ResponseWriter, r *http.Request) {
	body, err := io.ReadAll(r.Body)
	if err != nil {
		v.t.Errorf("vendor request body: %v", err)
		return
	}
	v.mu.Lock()
	v.requests = append(
		v.requests,
		RecordedRequest{Method: r.Method, URI: r.URL.RequestURI(), Headers: r.Header.Clone(), Body: body},
	)
	close(v.changed)
	v.changed = make(chan struct{})
	response := MockResponse{Status: 500, Chunks: [][]byte{[]byte("MockVendor: no response scripted")}}
	if route := v.routes[r.URL.Path]; len(route) > 0 {
		response = route[0]
		v.routes[r.URL.Path] = route[1:]
	} else if len(v.responses) > 0 {
		response = v.responses[0]
		v.responses = v.responses[1:]
	}
	v.mu.Unlock()
	v.writeResponse(ctx, w, r, response)
}

// AssertBuiltinCatalog checks a built-in API-key provider without inference or network IO.
func AssertBuiltinCatalog(ctx context.Context, t testing.TB, p provider.Provider, vendor *MockVendor) {
	t.Helper()
	if p.Capabilities().ProcessHost {
		t.Error("API-key entry starts a process")
	}
	if _, ok := p.AuthStatus(ctx).(*types.Authenticated); !ok {
		t.Error("provider is not authenticated")
	}
	if _, ok := p.RuntimeState().(*types.RuntimeReady); !ok {
		t.Error("provider is not ready")
	}
	list, err := p.ListModels(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if list.SourceFetchedAt != types.Timestamp("1970-01-01T00:00:00.000Z") || list.Stale {
		t.Error("built-in catalog was marked fetched or stale")
	}
	found := false
	for _, model := range list.Models {
		if list.DefaultModelID != nil && *list.DefaultModelID == model.ID {
			found = true
		}
	}
	if !found {
		t.Error("default is not a catalog model")
	}
	if len(vendor.Requests()) != 0 {
		t.Error("reading status or catalog made a request")
	}
}

func (v *MockVendor) writeResponse(ctx context.Context, w http.ResponseWriter, r *http.Request, response MockResponse) {
	if response.Ending == Silent {
		select {
		case <-ctx.Done():
		case <-r.Context().Done():
		}
		return
	}
	for name, values := range response.Headers {
		for _, value := range values {
			w.Header().Add(name, value)
		}
	}
	w.WriteHeader(response.Status)
	controller := http.NewResponseController(w)
	if err := controller.Flush(); err != nil {
		return
	}
	for _, chunk := range response.Chunks {
		if _, err := w.Write(chunk); err != nil {
			return
		}
		if err := controller.Flush(); err != nil {
			return
		}
	}
	switch response.Ending {
	case Complete, Silent:
	case Open:
		select {
		case <-ctx.Done():
		case <-r.Context().Done():
		}
		select {
		case v.disconnected <- struct{}{}:
		default:
		}
	case Broken:
		connection, _, err := controller.Hijack()
		if err != nil {
			v.t.Errorf("break vendor connection: %v", err)
			return
		}
		_ = connection.Close() // Deliberately break the connection without the terminating chunk.
	}
}
