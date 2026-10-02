package remotehosttest

import (
	"bufio"
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"testing"
)

// A real incomplete HTTP upload holds net/http's body mutex while cancelled.
// The server and client synchronize on the first byte, without timed waits.
func TestUploadReadCancellation(t *testing.T) {
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	done := make(chan error, 1)
	ready := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		b := pipeHTTPBody{r.Body, http.NewResponseController(w)}
		data := make([]byte, 1)
		if _, err := b.Read(ctx, data); err != nil {
			done <- err
			return
		}
		close(ready)
		_, err := b.Read(ctx, data)
		done <- err
	}))
	defer server.Close()
	conn, err := net.Dial("tcp", server.Listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := conn.Close(); err != nil {
			t.Error(err)
		}
	}()
	if _, err := fmt.Fprint(conn, "POST / HTTP/1.1\r\nHost: fixture\r\nContent-Length: 2\r\n\r\na"); err != nil {
		t.Fatal(err)
	}
	<-ready
	cancel()
	if err := <-done; err == nil {
		t.Fatal("incomplete cancelled upload read succeeded")
	}
	// Wait until the handler has returned and the response is available.
	response, err := http.ReadResponse(bufio.NewReader(conn), nil)
	if err != nil {
		t.Fatal(err)
	}
	_, _ = io.Copy(io.Discard, response.Body)
	if err := response.Body.Close(); err != nil {
		t.Fatal(err)
	}
}
