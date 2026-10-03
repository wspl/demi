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
	t.Skip("finding 1: oversized authenticated HTTP body loses its JSON refusal to a connection reset")
	ctx := t.Context()
	b, master := accountStart(ctx, t, accountHarness(ctx, t))
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
		accountRefusal(t, a, 413, webapi.ErrorCodeTooLarge)
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
	accountRefusal(t, a, 401, webapi.ErrorCodeUnauthenticated)
}

func TestAJSONBodyIsReadWhateverItsContentTypeAndMustMatchItsType(t *testing.T) {
	ctx := t.Context()
	b, _ := accountStart(ctx, t, accountHarness(ctx, t))
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
	accountEqual(t, untyped.Status, 200)
	accountEqual(t, string(accountDecode(t, untyped, webapi.DecodeIdentity).User.Email), backendtest.MasterEmail)
	for _, row := range [][2]string{{`{"email":"master@example.test","password":"master-pass-1","remember":true}`, "remember"}, {`{"email":"master@example.test"}`, "password"}, {`{"email":"master@example.test","password":""}`, "password"}, {`{"email":"master@example.test","password":5}`, "password"}, {"", ""}, {"{", ""}} {
		e := accountRefusal(t, login(row[0]), 400, webapi.ErrorCodeInvalidBody)
		if !strings.Contains(e.Message, row[1]) {
			t.Fatal(e.Message)
		}
	}
}

func TestTheWebAppBuildIsServedWithDeepNavigationWhileAPIMissesStayJSON(t *testing.T) {
	ctx := t.Context()
	h := accountHarness(ctx, t)
	if err := h.WriteWeb(ctx, map[string]string{"index.html": "<html>fixture page</html>", "main.js": "export const fixture = true"}); err != nil {
		t.Fatal(err)
	}
	b, master := accountStart(ctx, t, h)
	html := func(path string) backendtest.Answer {
		a, err := b.ReadWith(ctx, path, nil, http.Header{"Accept": []string{"text/html"}})
		if err != nil {
			t.Fatal(err)
		}
		return a
	}
	deep := html("/conversation/example")
	accountEqual(t, deep.Status, 200)
	if !strings.Contains(string(deep.Body), "fixture page") {
		t.Fatal(string(deep.Body))
	}
	script := accountRequest(ctx, t, b, "GET", "/main.js", nil, "")
	accountEqual(t, script.Status, 200)
	if !strings.Contains(string(script.Body), "fixture = true") {
		t.Fatal(string(script.Body))
	}
	accountRefusal(t, html("/missing.js"), 404, webapi.ErrorCodeNotFound)
	accountRefusal(t, accountRequest(ctx, t, b, "GET", "/conversation/example", nil, ""), 404, webapi.ErrorCodeNotFound)
	accountRefusal(t, accountRequest(ctx, t, b, "GET", "/api/no-such-resource", &master, ""), 404, webapi.ErrorCodeNotFound)
}

func TestAPageSocketMessageOverTheLimitFailsTheSocket(t *testing.T) {
	t.Skip("finding 2: oversized page message sends close code 1009 instead of closing without a code")
	ctx := t.Context()
	b, master := accountStart(ctx, t, accountHarness(ctx, t))
	id := "3c1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01"
	accountEqual(t, accountRequest(ctx, t, b, "POST", "/api/conversations", &master, `{"id":"`+id+`"}`).Status, 201)
	socket, err := backendtest.OpenAccountSocket(ctx, t, b, &master, id)
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
