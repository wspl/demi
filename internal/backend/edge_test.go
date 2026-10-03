package backend_test

import (
	"bufio"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/wspl/demi/internal/backend/backendtest"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/webapi"
)

func TestAJSONBodyOverItsLimitIsRefusedBeforeItIsRead(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	b, master, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	body := accountJSON(t, contract.Field{Name: "nickname", Value: strings.Repeat("x", 1024*1024)})
	for _, streamed := range []bool{false, true} {
		var reader io.Reader = strings.NewReader(body)
		if streamed {
			reader = struct{ io.Reader }{reader}
		}
		r, err := b.Response(ctx, "PATCH", "/api/auth/me", &master, http.Header{"Content-Type": []string{"application/json"}}, reader)
		if err != nil {
			t.Fatal(err)
		}
		a, err := backendtest.ReadAnswer(ctx, r)
		if err != nil {
			t.Fatal(err)
		}
		conversationEqual(t, a.Status, 413)
		conversationRefusal(t, a, webapi.ErrorCodeTooLarge)
	}
	conn, err := (&net.Dialer{}).DialContext(ctx, "tcp", b.Address().String())
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := conn.Close(); err != nil {
			t.Error(err)
		}
	}()
	if err := conn.SetDeadline(time.Now().Add(10 * time.Second)); err != nil {
		t.Fatal(err)
	}
	if _, err := fmt.Fprintf(conn, "PATCH /api/auth/me HTTP/1.1\r\nHost: %s\r\nContent-Type: application/json\r\nContent-Length: %d\r\nConnection: close\r\n\r\n", b.Address(), 1024*1024+1); err != nil {
		t.Fatal(err)
	}
	response, err := http.ReadResponse(bufio.NewReader(conn), nil)
	if err != nil {
		t.Fatal(err)
	}
	a, err := backendtest.ReadAnswer(ctx, response)
	if err != nil {
		t.Fatal(err)
	}
	conversationEqual(t, a.Status, 401)
	conversationRefusal(t, a, webapi.ErrorCodeUnauthenticated)
}

func TestAJSONBodyIsReadWhateverItsContentTypeAndMustMatchItsType(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	b, _, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	login := func(body string) backendtest.Answer {
		r, err := b.Response(ctx, "POST", "/api/auth/login", nil, nil, strings.NewReader(body))
		if err != nil {
			t.Fatal(err)
		}
		a, err := backendtest.ReadAnswer(ctx, r)
		if err != nil {
			t.Fatal(err)
		}
		return a
	}
	untyped := login(accountCredentials(t, backendtest.MasterEmail, backendtest.MasterPassword))
	conversationEqual(t, untyped.Status, 200)
	conversationEqual(t, string(conversationDecode(t, untyped, webapi.DecodeIdentity).User.Email), backendtest.MasterEmail)
	for _, row := range [][2]string{{`{"email":"master@example.test","password":"master-pass-1","remember":true}`, "remember"}, {`{"email":"master@example.test"}`, "password"}, {`{"email":"master@example.test","password":""}`, "password"}, {`{"email":"master@example.test","password":5}`, "password"}, {"", ""}, {"{", ""}} {
		eAnswer := login(row[0])
		conversationEqual(t, eAnswer.Status, 400)
		e := conversationDecode(t, eAnswer, webapi.DecodeErrorBody)
		conversationEqual(t, e.Code, webapi.ErrorCodeInvalidBody)
		if !strings.Contains(e.Message, row[1]) {
			t.Fatal(e.Message)
		}
	}
}

func TestTheWebAppBuildIsServedWithDeepNavigationWhileAPIMissesStayJSON(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	if err := h.WriteWeb(ctx, map[string]string{"index.html": "<html>fixture page</html>", "main.js": "export const fixture = true"}); err != nil {
		t.Fatal(err)
	}
	b, master, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	html := func(path string) backendtest.Answer {
		a, err := b.ReadWith(ctx, path, nil, http.Header{"Accept": []string{"text/html"}})
		if err != nil {
			t.Fatal(err)
		}
		return a
	}
	deep := html("/conversation/example")
	conversationEqual(t, deep.Status, 200)
	if !strings.Contains(string(deep.Body), "fixture page") {
		t.Fatal(string(deep.Body))
	}
	script := conversationRequest(ctx, t, b, nil, "GET", "/main.js", "", 200)
	if !strings.Contains(string(script.Body), "fixture = true") {
		t.Fatal(string(script.Body))
	}
	{
		answer := html("/missing.js")
		conversationEqual(t, answer.Status, 404)
		conversationRefusal(t, answer, webapi.ErrorCodeNotFound)
	}
	conversationRefusal(t, conversationRequest(ctx, t, b, nil, "GET", "/conversation/example", "", 404), webapi.ErrorCodeNotFound)
	conversationRefusal(t, conversationRequest(ctx, t, b, &master, "GET", "/api/no-such-resource", "", 404), webapi.ErrorCodeNotFound)
}

func TestAPageSocketMessageOverTheLimitFailsTheSocket(t *testing.T) {
	t.Parallel()
	ctx, h := conversationHarness(t)
	b, master, err := h.StartSetUp(ctx, t)
	wireMust(t, err)
	id := "3c1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01"
	conversationCreate(ctx, t, b, &master, id)
	socket, err := b.Conversation(ctx, t, &master, id)
	if err != nil {
		t.Fatal(err)
	}
	if err := socket.Send(ctx, &framewire.SendFrame{MessageID: "m1", Content: []framewire.ClientContent{&framewire.TextContent{Text: strings.Repeat("x", 1024*1024)}}}); err != nil {
		t.Fatal(err)
	}
	_, err = socket.Next(ctx)
	if err == nil {
		t.Fatal("oversized frame accepted")
	}
	if websocket.CloseStatus(err) != -1 {
		t.Fatalf("socket closed with code: %v", err)
	}
}
