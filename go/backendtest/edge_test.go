package backendtest_test

import (
	"bufio"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"testing"

	"github.com/wspl/demi/go/backendtest"
)

// jsonBodyLimit is the most bytes of a JSON body the backend reads.
const jsonBodyLimit = 1024 * 1024

func nicknameOverTheLimit() string {
	return `{"nickname":"` + strings.Repeat("x", jsonBodyLimit) + `"}`
}

// oversized sends a PATCH of the caller's account with body, chunked when
// chunked and with its length declared otherwise, over a connection of its own,
// and answers the refusal the backend gives. A session that is nil sends no
// cookie. The backend answers and closes before it has read the whole body,
// which an HTTP client reports as a failed write instead of the answer that
// came first, when it reports it at all.
func oversized(t *testing.T, b *backendtest.Backend, session *backendtest.Session, body string, chunked bool) (int, string) {
	t.Helper()
	host := strings.TrimPrefix(b.URL, "http://")
	conn, err := net.Dial("tcp", host)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.Close()
	head := "PATCH /api/auth/me HTTP/1.1\r\nHost: " + host + "\r\nContent-Type: application/json\r\n"
	if session != nil {
		head += "Cookie: " + session.Cookie + "\r\n"
	}
	payload := body
	if chunked {
		head += "Transfer-Encoding: chunked\r\n"
		payload = fmt.Sprintf("%x\r\n%s\r\n0\r\n\r\n", len(body), body)
	} else {
		head += fmt.Sprintf("Content-Length: %d\r\n", len(body))
	}
	go func() {
		// The backend stops reading once the body is over its limit, so this
		// write may fail; the answer is what the scenario reads.
		_, _ = conn.Write([]byte(head + "\r\n" + payload))
	}()
	response, err := http.ReadResponse(bufio.NewReader(conn), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	encoded, err := io.ReadAll(response.Body)
	if err != nil {
		t.Fatal(err)
	}
	answer := backendtest.Answer{Status: response.StatusCode, Body: encoded}
	return answer.Status, answer.ErrorCode()
}

// Cost: one backend, about a second, and a megabyte sent twice.
func TestAJSONBodyOverItsLimitIsRefusedBeforeItIsRead(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()

	// With its length declared up front.
	if status, code := oversized(t, b, master, nicknameOverTheLimit(), false); status != http.StatusRequestEntityTooLarge || code != "too_large" {
		t.Fatalf("a declared body over its limit is %d %s", status, code)
	}
	// Without one, the body is counted as it arrives.
	if status, code := oversized(t, b, master, nicknameOverTheLimit(), true); status != http.StatusRequestEntityTooLarge || code != "too_large" {
		t.Fatalf("a streamed body over its limit is %d %s", status, code)
	}
	// The gate runs first: without a session the same body is 401.
	if status, code := oversized(t, b, nil, nicknameOverTheLimit(), false); status != http.StatusUnauthorized || code != "unauthenticated" {
		t.Fatalf("a body over its limit without a session is %d %s", status, code)
	}
	b.Stop()
}

// Cost: one backend, about a second.
func TestAJSONBodyIsReadWhateverItsContentTypeAndMustMatchItsType(t *testing.T) {
	t.Parallel()
	b, _ := backendtest.New(t).StartSetUp()
	login := func(body string) *backendtest.Answer {
		return b.Do(backendtest.Request{Method: http.MethodPost, Path: "/api/auth/login", Raw: []byte(body)})
	}

	untyped := login(`{"email":"` + backendtest.MasterEmail + `","password":"` + backendtest.MasterPassword + `"}`)
	untyped.Expect(http.StatusOK)
	if untyped.Str("user.email") != backendtest.MasterEmail {
		t.Fatalf("the login answers %s", untyped.Body)
	}

	for _, refusal := range []struct{ body, field string }{
		{`{"email":"` + backendtest.MasterEmail + `","password":"` + backendtest.MasterPassword + `","remember":true}`, "remember"},
		{`{"email":"` + backendtest.MasterEmail + `"}`, "password"},
		{`{"email":"` + backendtest.MasterEmail + `","password":""}`, "password"},
		{`{"email":"` + backendtest.MasterEmail + `","password":5}`, "password"},
		{``, ""},
		{`{`, ""},
	} {
		refused := login(refusal.body)
		status, code := refused.Refusal()
		if status != http.StatusBadRequest || code != "invalid_body" {
			t.Fatalf("%s is %d %s", refusal.body, status, code)
		}
		if !strings.Contains(refused.ErrorMessage(), refusal.field) {
			t.Fatalf("%s: the message %q does not name %q", refusal.body, refused.ErrorMessage(), refusal.field)
		}
	}
	b.Stop()
}

// Cost: one backend, about a second.
func TestTheBrowserBuildIsServedWithDeepNavigationWhileAPIMissesStayJSON(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t, backendtest.WithWeb(map[string]string{
		"index.html": "<html>fixture page</html>",
		"main.js":    "export const fixture = true",
	}))
	b, master := h.StartSetUp()
	html := func(path string) *backendtest.Answer {
		return b.Do(backendtest.Request{Path: path, Headers: map[string]string{"Accept": "text/html"}})
	}

	deep := html("/conversation/example").Expect(http.StatusOK)
	if !strings.Contains(deep.Text(), "fixture page") {
		t.Fatalf("a deep link answers %s", deep.Body)
	}
	script := b.Get("/main.js", nil).Expect(http.StatusOK)
	if !strings.Contains(script.Text(), "fixture = true") {
		t.Fatalf("the script answers %s", script.Body)
	}

	for name, answer := range map[string]*backendtest.Answer{
		"a missing asset":   html("/missing.js"),
		"a path not a page": b.Get("/conversation/example", nil),
		"an API miss":       b.Get("/api/no-such-resource", master),
	} {
		if status, code := answer.Refusal(); status != http.StatusNotFound || code != "not_found" {
			t.Fatalf("%s is %d %s", name, status, code)
		}
	}
	b.Stop()
}
