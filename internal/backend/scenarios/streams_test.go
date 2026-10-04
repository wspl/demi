package scenarios_test

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
	"github.com/wspl/demi/internal/webapiproto"
)

// conversationStream opens a native user stream owned by the test.
func conversationStream(
	ctx context.Context,
	t *testing.T,
	backend *backendtest.TestBackend,
	session *backendtest.Session,
	id, name string,
) *websocket.Conn {
	t.Helper()
	socket, response, err := websocket.Dial(
		ctx,
		backend.WSURL("/api/conversations/"+id+"/streams/"+name),
		&websocket.DialOptions{
			HTTPClient: backend.HTTP,
			HTTPHeader: http.Header{"Origin": {backend.URL}, "Cookie": {session.Cookie}},
		},
	)
	if err != nil {
		if response != nil && response.Body != nil {
			wireMust(t, response.Body.Close())
		}
		t.Fatal(err)
	}
	socket.SetReadLimit(64 << 20)
	t.Cleanup(func() {
		_ = socket.CloseNow()
	}) // Teardown may follow a received close.
	return socket
}

// conversationStreamEnd collects stream bytes until its explicit close.
func conversationStreamEnd(ctx context.Context, t *testing.T, stream *websocket.Conn) ([]byte, int, string) {
	t.Helper()
	var data []byte
	for {
		kind, chunk, err := stream.Read(ctx)
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
func conversationStreamReady(ctx context.Context, t *testing.T, stream *websocket.Conn) {
	t.Helper()
	wireMust(t, stream.Write(ctx, websocket.MessageBinary, []byte("ping")))
	kind, data, err := stream.Read(ctx)
	wireMust(t, err)
	conversationEqual(t, kind, websocket.MessageBinary)
	conversationEqual(t, string(data), "ping")
}

// conversationStreamDevice selects a paired runner through the target HTTP route.
func conversationStreamDevice(
	t *testing.T,
) (context.Context, *backendtest.TestBackend, backendtest.Session, *backendtest.Paired) {
	t.Helper()
	ctx, harness := conversationHarness(t)
	built, err := backendtest.BuildPackage(ctx, t, "demi-native-fixture")
	wireMust(t, err)
	wireMust(t, harness.UseNativeFixture(ctx, built))
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	paired, err := backend.Pair(ctx, t, &session, "laptop")
	wireMust(t, err)
	body := `{"target":{"kind":"device","deviceId":"` + string(
		paired.ID(),
	) + `","path":` + conversationJSON(
		t,
		paired.Runner.Home(),
	) + `}}`
	conversationRequest(ctx, t, backend, &session, "PATCH", "/api/conversations/"+conversationFirst, body, 200)
	return ctx, backend, session, paired
}

// TestUserStreamCarriesContextDirectoryAndBytes checks that native streams carry their context,
// working directory and bytes.
// The real native fixture reports context, then echoes three MiB bidirectionally.
func TestUserStreamCarriesContextDirectoryAndBytes(t *testing.T) {
	t.Parallel()
	ctx, backend, session, paired := conversationStreamDevice(t)
	bytes, code, reason := conversationStreamEnd(
		ctx,
		t,
		conversationStream(ctx, t, backend, &session, conversationFirst, "where"),
	)
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
	conversationRequest(ctx, t, backend, &session, "PATCH", "/api/settings/preferences", `{"locale":`+locale+`}`, 200)
	bytes, _, _ = conversationStreamEnd(
		ctx,
		t,
		conversationStream(ctx, t, backend, &session, conversationFirst, "where"),
	)
	fields, err = contract.Object(bytes)
	wireMust(t, err)
	contextFields, err = contract.Object(fields["context"])
	wireMust(t, err)
	conversationEqual(t, string(contextFields["locale"]), locale)
	echo := conversationStream(ctx, t, backend, &session, conversationFirst, "echo")
	payload := backendtest.Pattern(3*1024*1024, 0)
	sendCtx, cancel := context.WithCancel(ctx)
	sent := make(chan error, 1)
	go func() {
		for offset := 0; offset < len(payload); offset += 100000 {
			if err := echo.Write(
				sendCtx,
				websocket.MessageBinary,
				payload[offset:min(offset+100000, len(payload))],
			); err != nil {
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

// TestUserStreamRefusesForeignUnknownArchivedAndOffline checks that streams refuse foreign, unknown,
// archived and offline conversations.
func TestUserStreamRefusesForeignUnknownArchivedAndOffline(t *testing.T) {
	t.Parallel()
	ctx, backend, session, paired := conversationStreamDevice(t)
	path := "/api/conversations/" + conversationFirst + "/streams/"
	conversationUpgradeRefusal(
		ctx,
		t,
		backend,
		&session,
		path+"echo",
		"https://elsewhere.example",
		403,
		webapiproto.ErrorCodeForbiddenOrigin,
	)
	conversationUpgradeRefusal(
		ctx,
		t,
		backend,
		&session,
		path+"browser",
		backend.URL,
		404,
		webapiproto.ErrorCodeUnknownStream,
	)
	conversationRefusal(
		t,
		conversationRequest(ctx, t, backend, &session, "GET", path+"echo", "", 426),
		webapiproto.ErrorCodeUpgradeRequired,
	)
	wireMust(t, paired.Runner.Kill(ctx))
	wireMust(t, backend.UntilOnline(ctx, &session, paired.ID(), false))
	conversationUpgradeRefusal(
		ctx,
		t,
		backend,
		&session,
		path+"echo",
		backend.URL,
		409,
		webapiproto.ErrorCodeDeviceOffline,
	)
	wireMust(t, paired.Runner.StartAgain(ctx))
	wireMust(t, backend.UntilOnline(ctx, &session, paired.ID(), true))
	conversationRequest(
		ctx,
		t,
		backend,
		&session,
		"PATCH",
		"/api/conversations/"+conversationFirst,
		`{"archived":true}`,
		200,
	)
	conversationUpgradeRefusal(
		ctx,
		t,
		backend,
		&session,
		path+"echo",
		backend.URL,
		409,
		webapiproto.ErrorCodeConversationArchived,
	)
}

// TestArchiveEndsOpenUserStreams checks that archiving closes existing user streams.
func TestArchiveEndsOpenUserStreams(t *testing.T) {
	t.Parallel()
	ctx, backend, session, _ := conversationStreamDevice(t)
	echo := conversationStream(ctx, t, backend, &session, conversationFirst, "echo")
	conversationStreamReady(ctx, t, echo)
	conversationRequest(
		ctx,
		t,
		backend,
		&session,
		"PATCH",
		"/api/conversations/"+conversationFirst,
		`{"archived":true}`,
		200,
	)
	_, code, reason := conversationStreamEnd(ctx, t, echo)
	conversationEqual(t, code, 4000)
	conversationEqual(t, reason, "conversation_changed")
}

// TestDisablingPluginEndsStreamsAndRefusesNewOnes checks that disabling a plugin closes its streams
// and refuses new ones.
func TestDisablingPluginEndsStreamsAndRefusesNewOnes(t *testing.T) {
	t.Parallel()
	ctx, backend, session, _ := conversationStreamDevice(t)
	echo := conversationStream(ctx, t, backend, &session, conversationFirst, "echo")
	conversationStreamReady(ctx, t, echo)
	conversationRequest(ctx, t, backend, &session, "PUT", "/api/plugins/fixture", `{"enabled":false}`, 204)
	_, code, reason := conversationStreamEnd(ctx, t, echo)
	conversationEqual(t, code, 4001)
	conversationEqual(t, reason, "plugin_disabled")
	conversationUpgradeRefusal(
		ctx,
		t,
		backend,
		&session,
		"/api/conversations/"+conversationFirst+"/streams/echo",
		backend.URL,
		404,
		webapiproto.ErrorCodeUnknownStream,
	)
}

// TestOpenUserStreamKeepsCloudAwakeUntilClose checks that an open user stream keeps its Cloud awake
// until closure.
// A native stream spans two 400-ms idle windows, observed by a page heartbeat.
func TestOpenUserStreamKeepsCloudAwakeUntilClose(t *testing.T) {
	t.Parallel()
	ctx := t.Context()
	harness, manager, err := backendtest.HostsHarness(ctx, t)
	wireMust(t, err)
	built, err := backendtest.BuildPackage(ctx, t, "demi-native-fixture")
	wireMust(t, err)
	wireMust(t, harness.UseNativeFixture(ctx, built))
	const window = 400 * time.Millisecond
	harness.Config.Lifecycle.IdleWindow = window
	harness.Config.Lifecycle.IdlePoll = 50 * time.Millisecond
	harness.Config.Cloud.Sweep = 50 * time.Millisecond
	harness.Config.Pages.Heartbeat = 2 * window
	backend, session, err := harness.StartSetUp(ctx, t)
	wireMust(t, err)
	conversationCreate(ctx, t, backend, &session, conversationFirst)
	conversationRequest(ctx, t, backend, &session, "GET", "/api/conversations/"+conversationFirst+"/fs", "", 200)
	var device string
	for _, call := range manager.Calls() {
		if strings.HasPrefix(call, "wake:") {
			device = strings.TrimPrefix(call, "wake:")
		}
	}
	if device == "" {
		t.Fatal("cloud never woke")
	}
	echo := conversationStream(ctx, t, backend, &session, conversationFirst, "echo")
	conversationStreamReady(ctx, t, echo)
	page, _ := conversationPage(ctx, t, backend, &session)
	_, err = page.Until(ctx, func(e webapiproto.SyncEvent) bool {
		_, ok := e.(*webapiproto.SyncEventHeartbeat)
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
