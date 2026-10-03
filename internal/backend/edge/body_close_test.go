package edge

import (
	"bufio"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/webapi"
)

// Two loopback sockets exercise refusal while an upload is still arriving.
// No sleeps or external services; protocol events order the writes and response.
func TestOversizedStreamingBodyReceivesRefusal(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	edge := &Edge{
		state: AppState{Services: &usershard.Services{}},
		handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			_, err := readJSONBody(r)
			if err != nil {
				writeError(w, err)
			}
		}),
	}
	done := make(chan struct{})
	go func() {
		defer close(done)
		conn, err := listener.Accept()
		if err != nil {
			return
		}
		ctx, incoming := readIncoming(t.Context(), conn)
		defer func() { _ = incoming.Close() }()
		edge.serveConnection(ctx, newActivity(incoming))
	}()
	defer func() {
		_ = listener.Close()
		<-done
	}()
	conn, err := (&net.Dialer{}).DialContext(t.Context(), "tcp", listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = conn.Close() }()
	if _, err := fmt.Fprint(
		conn,
		"PATCH / HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n",
	); err != nil {
		t.Fatal(err)
	}
	if _, err := fmt.Fprintf(conn, "%x\r\n%s\r\n", jsonBodyLimit+1, strings.Repeat("x", jsonBodyLimit+1)); err != nil {
		t.Fatal(err)
	}
	// Read only the response head. The upload has not ended; finish sending it
	// before reading the refusal body, as a client writer may still be doing.
	response, err := http.ReadResponse(bufio.NewReader(conn), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = response.Body.Close() }()
	if _, err := io.WriteString(conn, "400000\r\n"+strings.Repeat("x", 4*1024*1024)+"\r\n"); err != nil {
		t.Fatalf("upload reset before refusal could be read: %v", err)
	}
	if _, err := io.WriteString(conn, "0\r\n\r\n"); err != nil {
		t.Fatalf("upload reset before refusal could be read: %v", err)
	}
	body, err := io.ReadAll(response.Body)
	if err != nil {
		t.Fatal(err)
	}
	failure, err := webapi.DecodeErrorBody(body)
	if err != nil || response.StatusCode != 413 || failure.Code != webapi.ErrorCodeTooLarge {
		t.Fatal(response.StatusCode, string(body), err)
	}
}
