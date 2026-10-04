package httpserver_test

import (
	"bufio"
	"context"
	"io"
	"net"
	"net/http"
	"net/netip"
	"path/filepath"
	"testing"
	"time"

	"github.com/wspl/demi/internal/backend/httpserver"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/runnerproto"

	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/backend/usershard"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

func TestRequestAfterShutdownAnswersBackendClosing(t *testing.T) {
	control, err := database.OpenControl(t.Context(), filepath.Join(t.TempDir(), "control.sqlite"), types.SystemClock{})
	if err != nil {
		t.Fatal(err)
	}
	// The operation reports IO failures; cleanup has no further recipient.
	defer func() { _ = control.Close(context.Background()) }()
	state := httpserver.AppState{
		Services: &usershard.Services{Control: control, PublicURL: &runners.PublicURL{}},
		Site:     &httpserver.Site{},
	}
	e, err := httpserver.Start(t.Context(), netip.MustParseAddrPort("127.0.0.1:0"), state, "")
	if err != nil {
		t.Fatal(err)
	}
	// The operation reports IO failures; cleanup has no further recipient.
	defer func() { _ = e.Close(context.Background()) }()
	transport := &http.Transport{MaxConnsPerHost: 1}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport}
	endpoint := "http://" + e.LocalAddr().String() + "/api/setup"
	answer, err := client.Get(endpoint)
	if err != nil {
		t.Fatal(err)
	}
	body, err := io.ReadAll(answer.Body)
	_ = answer.Body.Close()
	if err != nil {
		t.Fatal(err)
	}
	if answer.StatusCode != 200 || string(body) != "{\"needed\":true}" {
		t.Fatal(answer.StatusCode, string(body))
	}
	e.StopAccepting()
	answer, err = client.Get(endpoint)
	if err != nil {
		t.Fatal(err)
	}
	body, err = io.ReadAll(answer.Body)
	_ = answer.Body.Close()
	if err != nil {
		t.Fatal(err)
	}
	failure, err := webapiproto.DecodeErrorBody(body)
	if err != nil || answer.StatusCode != 503 || failure.Code != webapiproto.ErrorCodeBackendClosing {
		t.Fatal(answer.StatusCode, string(body), err)
	}
	if conn, err := net.Dial("tcp", e.LocalAddr().String()); err == nil {
		_ = conn.Close()
		t.Fatal("listener still accepts")
	}
}

func TestRunnerProtocolRefusalUsesUpgradedListener(t *testing.T) {
	state := httpserver.AppState{
		Services: &usershard.Services{
			PublicURL: &runners.PublicURL{},
			Runners:   usershard.RunnerTuning{HelloDeadline: time.Minute},
		},
		Site: &httpserver.Site{},
	}
	e, err := httpserver.Start(t.Context(), netip.MustParseAddrPort("127.0.0.1:0"), state, "")
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := e.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	socket, _, err := websocket.Dial(t.Context(), "ws://"+e.LocalAddr().String()+"/api/runner", nil)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = socket.CloseNow() }()
	hello := &runnerproto.Hello{
		Protocol: 0,
		Runner: runnerproto.Info{
			Name:     "fixture",
			Platform: "linux",
			Version:  "fixture",
			Identity: runnerproto.HostIdentity{Hostname: "fixture", HomeDir: "/home/fixture"},
		},
	}
	bytes, err := runnerproto.Encode(hello)
	if err != nil {
		t.Fatal(err)
	}
	if err := socket.Write(t.Context(), websocket.MessageBinary, bytes); err != nil {
		t.Fatal(err)
	}
	kind, bytes, err := socket.Read(t.Context())
	if err != nil || kind != websocket.MessageBinary {
		t.Fatal(kind, err)
	}
	message, err := runnerproto.DecodeInbound(bytes)
	if err != nil {
		t.Fatal(err)
	}
	refusal, ok := message.(*runnerproto.HelloError)
	if !ok || refusal.Code != runnerproto.HelloErrorCodeUnsupportedProtocol ||
		refusal.Reason != "unsupported protocol 0; this backend speaks 24" {
		t.Fatalf("%#v", message)
	}
	// Reading the close acknowledges it, joining the server's refusal handshake.
	_, _, err = socket.Read(t.Context())
	if websocket.CloseStatus(err) != websocket.StatusNormalClosure {
		t.Fatal(err)
	}
}

func TestExposeHostnameIsSelectedBeforeHTTPHeaderParsing(t *testing.T) {
	domain, err := expose.ParseDomain("expose.example.test")
	if err != nil {
		t.Fatal(err)
	}
	state := httpserver.AppState{
		Services: &usershard.Services{PublicURL: &runners.PublicURL{}, ExposeDomain: &domain},
		Site:     &httpserver.Site{},
	}
	e, err := httpserver.Start(t.Context(), netip.MustParseAddrPort("127.0.0.1:0"), state, "")
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := e.Close(context.Background()); err != nil {
			t.Error(err)
		}
	}()
	conn, err := (&net.Dialer{}).DialContext(t.Context(), "tcp", e.LocalAddr().String())
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = conn.Close() }()
	// An unknown expose is answered by the relay before net/http can reject
	// the malformed header name. Product routing must never see this request.
	if _, err := io.WriteString(
		conn,
		"GET /api/setup HTTP/1.1\r\nhOsT: unknown.expose.example.test\r\nbad header: value\r\n\r\n",
	); err != nil {
		t.Fatal(err)
	}
	answer, err := http.ReadResponse(bufio.NewReader(conn), nil)
	if err != nil {
		t.Fatal(err)
	}
	body, err := io.ReadAll(answer.Body)
	_ = answer.Body.Close()
	if err != nil || answer.StatusCode != 404 ||
		string(body) != "This expose does not exist (anymore); its URL is gone or has expired.\n" {
		t.Fatal(answer.StatusCode, string(body), err)
	}
}
