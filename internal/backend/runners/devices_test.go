package runners_test

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"sync"
	"testing"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/database/databasetest"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/backend/runners/runnerstest"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/runnerwire"
)

// runnerFrames carries scripted runner frames without a socket. Channels are
// owned by the test; connection cancellation unblocks all driver operations.
type runnerFrames chan []byte

func (f runnerFrames) Receive(ctx context.Context) ([]byte, error) {
	select {
	case data, ok := <-f:
		if !ok {
			return nil, io.EOF
		}
		return data, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}
func (f runnerFrames) Send(ctx context.Context, data []byte) error {
	select {
	case f <- data:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}

// unavailableSeen deliberately closes storage to verify that last-seen failure
// never prevents a runner from going offline or its owner's pages from updating.
func unavailableSeen(t *testing.T) (*runners.LastSeen, *pagesync.Registration) {
	t.Helper()
	control := databasetest.Control(t.Context(), t, nil)
	if err := control.Close(t.Context()); err != nil {
		t.Fatal(err)
	}
	registry := &pagesync.SyncRegistry{}
	registration := registry.Register("user", database.HashToken("session"))
	t.Cleanup(registration.Release)
	return runners.NewLastSeen(control, registry.Of("user")), registration
}

func TestDeviceConnectionLifecycleWithUnavailableLastSeen(t *testing.T) {
	seen, pages := unavailableSeen(t)
	pipes := remotehost.NewPipes(remotehost.Arrival)
	t.Cleanup(func() {
		if err := pipes.Close(context.Background()); err != nil {
			t.Error(err)
		}
	})
	var devices runners.Devices
	gate := runners.NewFileGate("conversation")
	lease, err := gate.Enter(t.Context(), 0)
	if err != nil {
		t.Fatal(err)
	}
	defer lease.Release()
	h := devices.ConversationHost("device", lease, "/work", nil)
	if h.Online() {
		t.Fatal("unconnected host online")
	}
	connect := func(home string) (*remotehost.Link, *runnerstest.Connection, runnerFrames) {
		t.Helper()
		link, driver := remotehost.NewLink(remotehost.LinkOptions{Device: "device", Identity: host.Identity{HomeDir: home}, Pipes: pipes, Policy: remotehosttest.NewCommandPolicy(nil)})
		serving := devices.Bind("device", link, driver, seen)
		incoming := make(runnerFrames, 8)
		connection := runnerstest.Serve(t, serving, incoming, make(runnerFrames, 64))
		return link, connection, incoming
	}
	link, connection, incoming := connect("/first")
	if err := devices.UntilOnline(t.Context(), "device"); err != nil {
		t.Fatal(err)
	}
	if !h.Online() || h.Identity().HomeDir != "/first" || devices.DeviceAccess("device") == nil {
		t.Fatal("bound host unavailable")
	}
	installs := []runnerwire.Install{{Package: "example.commands", Name: "Commands", Version: "1", Phase: runnerwire.InstallPhaseDownload, Done: 1, Total: 2}}
	frame, err := runnerwire.Encode(&runnerwire.Installs{Installs: installs})
	if err != nil {
		t.Fatal(err)
	}
	if err := incoming.Send(t.Context(), frame); err != nil {
		t.Fatal(err)
	}
	if err := pages.Marked(t.Context()); err != nil {
		t.Fatal(err)
	}
	pages.Take()
	if !reflect.DeepEqual(link.Installs(), installs) {
		t.Fatal("installation report lost")
	}
	devices.Disconnect("device", "fixture disconnect")
	end, err := connection.End(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if end.Kind != remotehost.LinkDisconnected || end.Reason != "fixture disconnect" {
		t.Fatalf("end %+v", end)
	}
	if err := devices.Settled(t.Context(), "device"); err != nil {
		t.Fatal(err)
	}
	if h.Online() || h.Identity().HomeDir != "/first" || devices.DeviceAccess("device") != nil {
		t.Fatal("offline identity lost")
	}
	if err := pages.Marked(t.Context()); err != nil {
		t.Fatal(err)
	}
	pages.Take()
	_, second, _ := connect("/second")
	if !h.Online() || h.Identity().HomeDir != "/second" {
		t.Fatal("existing host did not follow reconnect")
	}
	devices.DisconnectAll("shutdown")
	if _, err := second.End(t.Context()); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if err := devices.UntilOnline(ctx, "device"); !errors.Is(err, context.Canceled) {
		t.Fatalf("cancel offline wait: %v", err)
	}
}

// connectionLog records messages safely while serving runs in another goroutine.
type connectionLog struct {
	mu       sync.Mutex
	messages []string
}

func (*connectionLog) Enabled(context.Context, slog.Level) bool { return true }
func (l *connectionLog) Handle(_ context.Context, r slog.Record) error {
	l.mu.Lock()
	l.messages = append(l.messages, r.Message)
	l.mu.Unlock()
	return nil
}
func (l *connectionLog) WithAttrs([]slog.Attr) slog.Handler { return l }
func (l *connectionLog) WithGroup(string) slog.Handler      { return l }

func TestServingRevocationAndProtocolLogging(t *testing.T) {
	for _, mode := range []string{"revoked", "text", "malformed"} {
		t.Run(mode, func(t *testing.T) {
			seen, _ := unavailableSeen(t)
			log := &connectionLog{}
			previous := slog.Default()
			slog.SetDefault(slog.New(log))
			defer slog.SetDefault(previous)
			pipes := remotehost.NewPipes(remotehost.Arrival)
			defer func() {
				if err := pipes.Close(context.Background()); err != nil {
					t.Error(err)
				}
			}()
			var devices runners.Devices
			link, driver := remotehost.NewLink(remotehost.LinkOptions{Device: "device", Pipes: pipes, Policy: remotehosttest.NewCommandPolicy(nil)})
			serving := devices.Bind("device", link, driver, seen)
			done := make(chan remotehost.LinkEnd, 1)
			lifetime, cancel := context.WithCancel(t.Context())
			defer cancel()
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				socket, err := websocket.Accept(w, r, nil)
				if err != nil {
					t.Error(err)
					return
				}
				done <- serving.Serve(lifetime, socket)
			}))
			defer server.Close()
			socket, _, err := websocket.Dial(t.Context(), strings.Replace(server.URL, "http://", "ws://", 1), nil)
			if err != nil {
				t.Fatal(err)
			}
			defer func() { _ = socket.CloseNow() }() // Serving already ends the link on every path.
			if mode == "revoked" {
				devices.Revoke("device")
				kind, data, err := socket.Read(t.Context())
				if err != nil {
					t.Fatal(err)
				}
				if kind != websocket.MessageBinary {
					t.Fatal("nonbinary refusal")
				}
				message, err := runnerwire.DecodeInbound(data)
				if err != nil {
					t.Fatal(err)
				}
				refusal, ok := message.(*runnerwire.HelloError)
				if !ok || refusal.Code != runnerwire.HelloErrorCodeRevoked || refusal.Reason != "device revoked" {
					t.Fatalf("refusal %#v", message)
				}
			} else {
				kind, data := websocket.MessageText, []byte("text")
				if mode == "malformed" {
					kind, data = websocket.MessageBinary, []byte{0xc1}
				}
				if err := socket.Write(t.Context(), kind, data); err != nil {
					t.Fatal(err)
				}
			}
			// Keep reading so the close handshake is acknowledged, then join the server.
			_, _, _ = socket.Read(t.Context())
			end := <-done
			want := "runner connection ended: device revoked"
			if mode == "text" {
				want = "runner connection ended: runner disconnected: the runner sent a text frame"
				if end.Kind != remotehost.LinkClosed {
					t.Fatalf("end %+v", end)
				}
			}
			if mode == "malformed" {
				want = "runner connection closed: " + end.Reason
				if end.Kind != remotehost.LinkRefused {
					t.Fatalf("end %+v", end)
				}
			}
			log.mu.Lock()
			messages := append([]string(nil), log.messages...)
			log.mu.Unlock()
			found := false
			for _, message := range messages {
				if message == want {
					found = true
				}
			}
			if !found {
				t.Fatalf("missing Rust log %q: %v", want, messages)
			}
			if devices.Online("device") {
				t.Fatal("connection left online")
			}
		})
	}
}
