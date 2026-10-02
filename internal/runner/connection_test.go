package runner

import (
	"testing"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/runnerwire"
)

// Real local programs; about one second total, with no wall-time synchronization.
func TestSocketURL(t *testing.T) {
	for _, test := range []struct{ path, want string }{{"", "/api/runner"}, {"/custom?x=1", "/custom?x=1"}} {
		t.Run(test.want, func(t *testing.T) {
			f := newRunner(t, nil, test.path)
			if got := <-f.requested; got != test.want {
				t.Fatalf("route %q, want %q", got, test.want)
			}
		})
	}
}
func TestTypedExchangeAndRemoteClose(t *testing.T) {
	f := newRunner(t, nil, "")
	f.online()
	f.send(&runnerwire.Ping{})
	if pong, ok := f.frame().(*runnerwire.Pong); !ok || pong.Jobs != 0 {
		t.Fatal("ping did not report no jobs")
	}
	if err := f.socket.Close(websocket.StatusNormalClosure, ""); err != nil {
		t.Fatal(err)
	}
	f.accept()
	if f.hello.DeviceToken == nil || f.hello.DeviceToken.Expose() != "test-token" {
		t.Fatal("reconnect lost credential")
	}
}
func TestMalformedInputFailsConnection(t *testing.T) {
	f := newRunner(t, nil, "")
	f.online()
	if err := f.socket.Write(f.ctx, websocket.MessageText, []byte("{}")); err != nil {
		t.Fatal(err)
	}
	if _, _, err := f.socket.Read(f.ctx); err == nil {
		t.Fatal("malformed message kept connection open")
	}
	f.accept()
}
func TestFullInboundQueueWaits(t *testing.T) {
	f := newRunner(t, nil, "")
	f.online()
	for range 256 {
		f.send(&runnerwire.Ping{})
	}
	for range 256 {
		if _, ok := f.frame().(*runnerwire.Pong); !ok {
			t.Fatal("missing pong")
		}
	}
}
