package runner

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/runnerwire"
)

func transportPair(t *testing.T) (*transport, *websocket.Conn) {
	t.Helper()
	accepted := make(chan *websocket.Conn, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		socket, err := websocket.Accept(w, r, nil)
		if err != nil {
			t.Error(err)
			return
		}
		accepted <- socket
	}))
	t.Cleanup(server.Close)
	backend, err := runnerwire.ParseBackendURL(server.URL)
	if err != nil {
		t.Fatal(err)
	}
	client, err := connect(t.Context(), backend)
	if err != nil {
		t.Fatal(err)
	}
	peer := <-accepted
	t.Cleanup(func() {
		_ = peer.CloseNow()
		if err := client.close(context.Background()); err != nil {
			t.Logf("connection ended: %v", err)
		}
	})
	return client, peer
}

func TestTransportDrainFlushesBeforeGoingAway(t *testing.T) {
	client, peer := transportPair(t)
	for i := range 16 {
		frame, err := runnerwire.Encode(&runnerwire.Pong{Jobs: uint64(i)})
		if err != nil {
			t.Fatal(err)
		}
		client.control <- frame
	}
	closed := make(chan error, 1)
	go func() { closed <- client.close(context.Background()) }()
	defer func() {
		if err := <-closed; err != nil {
			t.Error(err)
		}
	}()
	for i := range 16 {
		_, data, err := peer.Read(t.Context())
		if err != nil {
			t.Fatal(err)
		}
		message, err := runnerwire.DecodeOutbound(data)
		if err != nil {
			t.Fatal(err)
		}
		pong, ok := message.(*runnerwire.Pong)
		if !ok || pong.Jobs != uint64(i) {
			t.Fatalf("frame %d: %#v", i, message)
		}
	}
	_, _, err := peer.Read(t.Context())
	if websocket.CloseStatus(err) != websocket.StatusGoingAway {
		t.Fatalf("close: %v", err)
	}
}
