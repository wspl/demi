package usershard

import (
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
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		socket, err := websocket.Accept(w, r, nil)
		if err != nil {
			result <- err
			return
		}
		defer socket.CloseNow()
		page := newPageSocket(socket, DefaultPageTuning())
		defer page.heartbeat.Stop()
		_, _, err = socket.Read(r.Context())
		result <- err
	}))
	defer server.Close()
	socket, _, err := websocket.Dial(t.Context(), "ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer socket.CloseNow()
	// The peer may close while Write is completing; the receiving refusal is the assertion.
	_ = socket.Write(t.Context(), websocket.MessageText, make([]byte, webapi.MaxPageMessageBytes+1))
	_, _, err = socket.Read(t.Context())
	if websocket.CloseStatus(err) != websocket.StatusMessageTooBig {
		t.Fatalf("oversized page message: %v", err)
	}
	if err := <-result; err == nil {
		t.Fatal("server accepted oversized message")
	}
}
