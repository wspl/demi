package backend_test

import (
	"context"
	"errors"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapi"
)

// conversationStream opens a native user stream owned by the test.
func conversationStream(ctx context.Context, t *testing.T, b *backendtest.TestBackend, s *backendtest.Session, id, name string) *websocket.Conn {
	t.Helper()
	socket, response, err := websocket.Dial(ctx, b.WSURL("/api/conversations/"+id+"/streams/"+name), &websocket.DialOptions{HTTPClient: b.HTTP, HTTPHeader: http.Header{"Origin": {b.URL}, "Cookie": {s.Cookie}}})
	if err != nil {
		if response != nil && response.Body != nil {
			wireMust(t, response.Body.Close())
		}
		t.Fatal(err)
	}
	socket.SetReadLimit(64 << 20)
	t.Cleanup(func() { _ = socket.CloseNow() }) // Teardown may follow a received close.
	return socket
}

// conversationStreamEnd collects stream bytes until its explicit close.
func conversationStreamEnd(ctx context.Context, t *testing.T, s *websocket.Conn) ([]byte, int, string) {
	t.Helper()
	var data []byte
	for {
		kind, chunk, err := s.Read(ctx)
		if err != nil {
			var closed websocket.CloseError
			if !errors.As(err, &closed) {
				t.Fatal(err)
			}
			return data, int(closed.Code), closed.Reason
		}
		if kind == websocket.MessageBinary {
			data = append(data, chunk...)
		}
	}
}

// conversationStreamReady observes an echo, proving native stream admission ended.
func conversationStreamReady(ctx context.Context, t *testing.T, s *websocket.Conn) {
	t.Helper()
	wireMust(t, s.Write(ctx, websocket.MessageBinary, []byte("ping")))
	kind, data, err := s.Read(ctx)
	wireMust(t, err)
	conversationEqual(t, kind, websocket.MessageBinary)
	conversationEqual(t, string(data), "ping")
}

// conversationStreamDevice selects a paired runner through the target HTTP route.
func conversationStreamDevice(t *testing.T) (context.Context, *backendtest.TestBackend, backendtest.Session, *backendtest.Paired) {
	t.Helper()
	ctx, h := conversationHarness(t)
	built, err := backendtest.BuildPackage(ctx, t, "demi-native-fixture")
	wireMust(t, err)
	wireMust(t, h.UseNativeFixture(ctx, built))
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationCreate(ctx, t, b, &s, conversationFirst)
	paired, err := b.Pair(ctx, t, &s, "laptop")
	wireMust(t, err)
	body := `{"target":{"kind":"device","deviceId":"` + string(paired.ID()) + `","path":` + conversationJSON(t, paired.Runner.Home()) + `}}`
	conversationRequest(ctx, t, b, &s, "PATCH", "/api/conversations/"+conversationFirst, body, 200)
	return ctx, b, s, paired
}

// The real native fixture reports context, then echoes three MiB bidirectionally.
func TestUserStreamCarriesContextDirectoryAndBytes(t *testing.T) {
	t.Parallel()
	ctx, b, s, paired := conversationStreamDevice(t)
	bytes, code, reason := conversationStreamEnd(ctx, t, conversationStream(ctx, t, b, &s, conversationFirst, "where"))
	conversationEqual(t, code, 1000)
	conversationEqual(t, reason, "completed")
	fields, err := contract.Object(bytes)
	wireMust(t, err)
	contextFields, err := contract.Object(fields["context"])
	wireMust(t, err)
	conversationEqual(t, string(contextFields["conversation"]), conversationJSON(t, conversationFirst))
	conversationEqual(t, string(contextFields["caller"]), `{"kind":"user"}`)
	conversationEqual(t, string(contextFields["locale"]), `{"timeZone":"UTC","languages":["en-US"]}`)
	conversationEqual(t, string(fields["cwd"]), conversationJSON(t, paired.Runner.Home()))
	conversationEqual(t, string(fields["value"]), "null")
	locale := `{"timeZone":"Asia/Shanghai","languages":["zh-CN","en"]}`
	conversationRequest(ctx, t, b, &s, "PATCH", "/api/settings/preferences", `{"locale":`+locale+`}`, 200)
	bytes, _, _ = conversationStreamEnd(ctx, t, conversationStream(ctx, t, b, &s, conversationFirst, "where"))
	fields, err = contract.Object(bytes)
	wireMust(t, err)
	contextFields, err = contract.Object(fields["context"])
	wireMust(t, err)
	conversationEqual(t, string(contextFields["locale"]), locale)
	echo := conversationStream(ctx, t, b, &s, conversationFirst, "echo")
	payload := backendtest.Pattern(3*1024*1024, 0)
	sendCtx, cancel := context.WithCancel(ctx)
	sent := make(chan error, 1)
	go func() {
		for offset := 0; offset < len(payload); offset += 100000 {
			if err := echo.Write(sendCtx, websocket.MessageBinary, payload[offset:min(offset+100000, len(payload))]); err != nil {
				sent <- err
				return
			}
		}
		sent <- nil
	}()
	defer func() {
		cancel()
		wireMust(t, <-sent)
	}()
	var echoed []byte
	for len(echoed) < len(payload) {
		kind, data, err := echo.Read(ctx)
		wireMust(t, err)
		conversationEqual(t, kind, websocket.MessageBinary)
		echoed = append(echoed, data...)
	}
	conversationEqual(t, echoed, payload)
	wireMust(t, echo.Close(websocket.StatusNormalClosure, ""))
}

func TestUserStreamRefusesForeignUnknownArchivedAndOffline(t *testing.T) {
	t.Parallel()
	ctx, b, s, paired := conversationStreamDevice(t)
	path := "/api/conversations/" + conversationFirst + "/streams/"
	conversationUpgradeRefusal(ctx, t, b, &s, path+"echo", "https://elsewhere.example", 403, webapi.ErrorCodeForbiddenOrigin)
	conversationUpgradeRefusal(ctx, t, b, &s, path+"browser", b.URL, 404, webapi.ErrorCodeUnknownStream)
	conversationRefusal(t, conversationRequest(ctx, t, b, &s, "GET", path+"echo", "", 426), webapi.ErrorCodeUpgradeRequired)
	wireMust(t, paired.Runner.Kill(ctx))
	wireMust(t, b.UntilOnline(ctx, &s, paired.ID(), false))
	conversationUpgradeRefusal(ctx, t, b, &s, path+"echo", b.URL, 409, webapi.ErrorCodeDeviceOffline)
	wireMust(t, paired.Runner.StartAgain(ctx))
	wireMust(t, b.UntilOnline(ctx, &s, paired.ID(), true))
	conversationRequest(ctx, t, b, &s, "PATCH", "/api/conversations/"+conversationFirst, `{"archived":true}`, 200)
	conversationUpgradeRefusal(ctx, t, b, &s, path+"echo", b.URL, 409, webapi.ErrorCodeConversationArchived)
}

func TestArchiveEndsOpenUserStreams(t *testing.T) {
	t.Parallel()
	ctx, b, s, _ := conversationStreamDevice(t)
	echo := conversationStream(ctx, t, b, &s, conversationFirst, "echo")
	conversationStreamReady(ctx, t, echo)
	conversationRequest(ctx, t, b, &s, "PATCH", "/api/conversations/"+conversationFirst, `{"archived":true}`, 200)
	_, code, reason := conversationStreamEnd(ctx, t, echo)
	conversationEqual(t, code, 4000)
	conversationEqual(t, reason, "conversation_changed")
}

func TestDisablingPluginEndsStreamsAndRefusesNewOnes(t *testing.T) {
	t.Parallel()
	ctx, b, s, _ := conversationStreamDevice(t)
	echo := conversationStream(ctx, t, b, &s, conversationFirst, "echo")
	conversationStreamReady(ctx, t, echo)
	conversationRequest(ctx, t, b, &s, "PUT", "/api/plugins/fixture", `{"enabled":false}`, 204)
	_, code, reason := conversationStreamEnd(ctx, t, echo)
	conversationEqual(t, code, 4001)
	conversationEqual(t, reason, "plugin_disabled")
	conversationUpgradeRefusal(ctx, t, b, &s, "/api/conversations/"+conversationFirst+"/streams/echo", b.URL, 404, webapi.ErrorCodeUnknownStream)
}

// A native stream spans two 400-ms idle windows, observed by a page heartbeat.
func TestOpenUserStreamKeepsCloudAwakeUntilClose(t *testing.T) {
	t.Parallel()
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	t.Cleanup(cancel)
	manager, err := backendtest.StartScriptedManager(ctx, t)
	wireMust(t, err)
	h, err := backendtest.NewHarness(ctx, t, manager.Socket())
	wireMust(t, err)
	built, err := backendtest.BuildPackage(ctx, t, "demi-native-fixture")
	wireMust(t, err)
	wireMust(t, h.UseNativeFixture(ctx, built))
	const window = 400 * time.Millisecond
	h.Config.Lifecycle.IdleWindow = window
	h.Config.Lifecycle.IdlePoll = 50 * time.Millisecond
	h.Config.Cloud.Sweep = 50 * time.Millisecond
	h.Config.Pages.Heartbeat = 2 * window
	b, s, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationCreate(ctx, t, b, &s, conversationFirst)
	conversationRequest(ctx, t, b, &s, "GET", "/api/conversations/"+conversationFirst+"/fs", "", 200)
	var device string
	for _, call := range manager.Calls() {
		if strings.HasPrefix(call, "wake:") {
			device = strings.TrimPrefix(call, "wake:")
		}
	}
	if device == "" {
		t.Fatal("cloud never woke")
	}
	echo := conversationStream(ctx, t, b, &s, conversationFirst, "echo")
	conversationStreamReady(ctx, t, echo)
	page, _ := conversationPage(ctx, t, b, &s)
	_, err = page.Until(ctx, func(e webapi.SyncEvent) bool {
		_, ok := e.(*webapi.SyncEventHeartbeat)
		return ok
	})
	wireMust(t, err)
	closed := time.Now()
	wireMust(t, echo.Close(websocket.StatusNormalClosure, ""))
	stopped, err := manager.Arrival(ctx, "hibernate:"+device)
	wireMust(t, err)
	if stopped.Before(closed.Add(window)) {
		t.Fatalf("cloud stopped %s after stream close", stopped.Sub(closed))
	}
}
