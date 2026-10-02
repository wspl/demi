package runner

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/runnerwire"
	"go.uber.org/goleak"
)

func TestMain(m *testing.M) { goleak.VerifyTestMain(m) }

// These transport scenarios use a local socket and cost no external resources.
func TestSocketURL(t *testing.T) {
	for _, test := range []struct{ backend, want string }{
		{"http://localhost", "ws://localhost/api/runner"},
		{"https://localhost/custom?x=1", "wss://localhost/custom?x=1"},
		{"ws://localhost/", "ws://localhost/api/runner"},
	} {
		backend, err := runnerwire.ParseBackendURL(test.backend)
		if err != nil {
			t.Fatal(err)
		}
		got, err := socketURL(backend)
		if err != nil || got != test.want {
			t.Fatalf("socketURL = %q, %v; want %q", got, err, test.want)
		}
	}
}

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

func TestTransportTypedExchangeAndRemoteClose(t *testing.T) {
	client, peer := transportPair(t)
	ping, err := runnerwire.Encode(&runnerwire.Ping{})
	if err != nil {
		t.Fatal(err)
	}
	if err := peer.Write(t.Context(), websocket.MessageBinary, ping); err != nil {
		t.Fatal(err)
	}
	if _, ok := (<-client.input).(*runnerwire.Ping); !ok {
		t.Fatal("expected ping")
	}
	pong, err := runnerwire.Encode(&runnerwire.Pong{Jobs: 0})
	if err != nil {
		t.Fatal(err)
	}
	client.control <- pong
	_, data, err := peer.Read(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	message, err := runnerwire.DecodeOutbound(data)
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := message.(*runnerwire.Pong); !ok {
		t.Fatalf("received %T", message)
	}
	if err := peer.Close(websocket.StatusNormalClosure, ""); err != nil {
		t.Fatal(err)
	}
	<-client.done
	if client.err != nil {
		t.Fatal(client.err)
	}
}

func TestTransportMalformedInput(t *testing.T) {
	for _, test := range []struct {
		name string
		kind websocket.MessageType
		data []byte
	}{
		{"text", websocket.MessageText, []byte("{}")},
		{"invalid messagepack", websocket.MessageBinary, []byte{0xc1}},
	} {
		t.Run(test.name, func(t *testing.T) {
			client, peer := transportPair(t)
			if err := peer.Write(t.Context(), test.kind, test.data); err != nil {
				t.Fatal(err)
			}
			<-client.done
			if client.err == nil {
				t.Fatal("malformed input accepted")
			}
		})
	}
}

func TestTransportFullInboundQueueWaits(t *testing.T) {
	client, peer := transportPair(t)
	ping, err := runnerwire.Encode(&runnerwire.Ping{})
	if err != nil {
		t.Fatal(err)
	}
	sent := make(chan error, 1)
	go func() {
		for range 256 {
			if err := peer.Write(t.Context(), websocket.MessageBinary, ping); err != nil {
				sent <- err
				return
			}
		}
		sent <- nil
	}()
	defer func() {
		if err := <-sent; err != nil {
			t.Error(err)
		}
	}()
	for range 256 {
		select {
		case message := <-client.input:
			if _, ok := message.(*runnerwire.Ping); !ok {
				t.Fatalf("received %T", message)
			}
		case <-client.done:
			t.Fatalf("queue closed connection: %v", client.err)
		}
	}
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
