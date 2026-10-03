package usershard

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/coder/websocket"

	"github.com/wspl/demi/internal/webapi"
)

// This uses one loopback socket, no model or timer wait.
func TestPageSocketRejectsOversizedMessage(t *testing.T) {
	result := make(chan error, 1)
	done := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer close(done)
		socket, err := websocket.Accept(w, r, nil)
		if err != nil {
			result <- err
			return
		}
		// The peer may have already closed the connection under test.
		defer func() {
			_ = socket.CloseNow()
		}()
		page := newPageSocket(socket, DefaultPageTuning())
		defer page.heartbeat.Stop()
		_, _, err = ReadPageMessage(r.Context(), socket)
		result <- err
	}))
	defer server.Close()
	socket, _, err := websocket.Dial(t.Context(), "ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	// The peer may have already closed the connection under test.
	defer func() {
		_ = socket.CloseNow()
		<-done
	}()
	// The peer may close while Write is completing; the receiving refusal is the assertion.
	_ = socket.Write(t.Context(), websocket.MessageText, make([]byte, webapi.MaxPageMessageBytes+1))
	_, _, err = socket.Read(t.Context())
	if err == nil || websocket.CloseStatus(err) != -1 {
		t.Fatalf("oversized page message: %v", err)
	}
	if err := <-result; !errors.Is(err, websocket.ErrMessageTooBig) {
		t.Fatalf("oversized message error: %v", err)
	}
}
