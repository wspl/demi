// This standalone regression uses only net/http; it does not import the SDK.
package http2reset

import (
	"bytes"
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestResetKeepsConnection(t *testing.T) {
	protocols := new(http.Protocols)
	protocols.SetUnencryptedHTTP2(true)
	read := make(chan struct{})
	server := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodGet {
			w.WriteHeader(http.StatusNoContent)
			return
		}
		defer close(read)
		// Leave the rest of the DATA frame buffered when the peer resets.
		if _, err := io.ReadFull(r.Body, make([]byte, 1)); err != nil {
			t.Error(err)
			return
		}
		if err := http.NewResponseController(w).Flush(); err != nil {
			t.Error(err)
			return
		}
		<-r.Context().Done()
		// Reading and closing a request body within its handler is valid even
		// if its peer has cancelled. No reads overlap, or outlive the handler.
		_, _ = io.Copy(io.Discard, r.Body)
	}))
	server.Config.Protocols = protocols
	server.Config.HTTP2 = &http.HTTP2Config{MaxReceiveBufferPerConnection: 1<<31 - 1}
	server.Start()
	defer server.Close()
	transport := &http.Transport{Protocols: protocols}
	defer transport.CloseIdleConnections()
	conn, err := transport.NewClientConn(t.Context(), "http", server.Listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, server.URL, bytes.NewReader(make([]byte, 4096)))
	if err != nil {
		t.Fatal(err)
	}
	response, err := conn.RoundTrip(req)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	cancel()
	<-read
	probe, err := http.NewRequestWithContext(t.Context(), http.MethodGet, server.URL, nil)
	if err != nil {
		t.Fatal(err)
	}
	response, err = conn.RoundTrip(probe)
	if err != nil {
		t.Fatalf("independent request after reset: %v", err)
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusNoContent {
		t.Fatalf("status = %d", response.StatusCode)
	}
}
