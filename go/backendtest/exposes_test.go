package backendtest_test

import (
	"bufio"
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"

	"github.com/wspl/demi/go/backendtest"
	"github.com/wspl/demi/go/backendtest/scripted"
)

// The visitors and the services of these scenarios write and read raw bytes, so
// the header case each side wrote is what the other reads. The services are
// fixtures on this machine, which is the device's network here.

const (
	exposeDomain = "expose.localhost"
	// exposeBodyBytes is the size of the bodies that cross the relay in each
	// direction.
	exposeBodyBytes = 8 << 20
	// exposeStep is how long a step may take before the scenario names what
	// never came.
	exposeStep      = 20 * time.Second
	exposeAgentConv = "5e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a01"
)

// A head is an HTTP message's head as its bytes had it: the first line, and each
// header line's name and value in their order, names in their case.
type head struct {
	line    string
	headers [][2]string
}

// lines are the header lines of name, whatever its case, in their order.
func (h head) lines(name string) [][2]string {
	var found [][2]string
	for _, line := range h.headers {
		if strings.EqualFold(line[0], name) {
			found = append(found, line)
		}
	}
	return found
}

// values are the values of name, whatever its case, in their order.
func (h head) values(name string) []string {
	var found []string
	for _, line := range h.lines(name) {
		found = append(found, line[1])
	}
	return found
}

// readHead reads a message head, up to and with the empty line.
func readHead(reader *bufio.Reader) (head, error) {
	var h head
	for first := true; ; first = false {
		text, err := reader.ReadString('\n')
		if err != nil {
			return h, fmt.Errorf("the stream ended in a head %q: %w", h.line, err)
		}
		text = strings.TrimSuffix(text, "\r\n")
		if first {
			h.line = text
			continue
		}
		if text == "" {
			return h, nil
		}
		name, value, found := strings.Cut(text, ":")
		if !found {
			return h, fmt.Errorf("a header line without a colon: %q", text)
		}
		h.headers = append(h.headers, [2]string{name, strings.TrimSpace(value)})
	}
}

// nextChunk reads the next chunk of a chunked body; it answers false at the last
// chunk, whose trailers are read too.
func nextChunk(reader *bufio.Reader) ([]byte, bool, error) {
	text, err := reader.ReadString('\n')
	if err != nil {
		return nil, false, err
	}
	size, err := strconv.ParseUint(strings.TrimSpace(text), 16, 32)
	if err != nil {
		return nil, false, fmt.Errorf("no chunk size in %q: %w", text, err)
	}
	if size == 0 {
		for {
			trailer, err := reader.ReadString('\n')
			if err != nil {
				return nil, false, err
			}
			if trailer == "\r\n" {
				return nil, false, nil
			}
		}
	}
	chunk := make([]byte, size+2)
	if _, err := io.ReadFull(reader, chunk); err != nil {
		return nil, false, err
	}
	if !bytes.HasSuffix(chunk, []byte("\r\n")) {
		return nil, false, fmt.Errorf("a chunk not ended by a line break")
	}
	return chunk[:size], true, nil
}

// readChunked reads a chunked body whole.
func readChunked(reader *bufio.Reader) ([]byte, error) {
	var body []byte
	for {
		chunk, more, err := nextChunk(reader)
		if err != nil || !more {
			return body, err
		}
		body = append(body, chunk...)
	}
}

// readChunksUntil reads the chunks of a chunked body until their text ends with
// end.
func readChunksUntil(reader *bufio.Reader, end string) (string, error) {
	var text strings.Builder
	for !strings.HasSuffix(text.String(), end) {
		chunk, more, err := nextChunk(reader)
		if err != nil {
			return text.String(), err
		}
		if !more {
			return text.String(), errors.New("the body ended")
		}
		text.Write(chunk)
	}
	return text.String(), nil
}

// writeChunked writes body as a chunked body, in chunks of 64 KiB.
func writeChunked(writer io.Writer, body []byte) error {
	for start := 0; start < len(body); start += 64 << 10 {
		chunk := body[start:min(start+64<<10, len(body))]
		if _, err := fmt.Fprintf(writer, "%x\r\n", len(chunk)); err != nil {
			return err
		}
		if _, err := writer.Write(chunk); err != nil {
			return err
		}
		if _, err := io.WriteString(writer, "\r\n"); err != nil {
			return err
		}
	}
	_, err := io.WriteString(writer, "0\r\n\r\n")
	return err
}

func asChunk(text string) string {
	return fmt.Sprintf("%x\r\n%s\r\n", len(text), text)
}

// What the HTTP fixture saw of one request.
type (
	seenRequest struct {
		head head
		body []byte
	}
	// seenEventStream is an event stream's answer, and whether its connection's
	// input stayed open while it streamed and ended after it.
	seenEventStream struct {
		openWhileStreaming bool
		endedAfterAnswer   bool
	}
	// seenReleased is a connection holding its answer open ended.
	seenReleased struct{}
)

// An httpFixture is a service on this machine that reads and writes raw bytes,
// by path: /headers answers with headers of mixed case and two cookies, /upload
// reads a chunked body whole and answers exposeBodyBytes chunked, /events
// streams two events, the second once proceed is sent, /refuse refuses an
// upgrade, /hello says hello, and /hold starts an answer it never finishes and
// tells when its connection ends.
type httpFixture struct {
	t       testing.TB
	port    int
	seen    chan any
	proceed chan struct{}
}

func startHTTPFixture(t testing.TB) *httpFixture {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	f := &httpFixture{
		t:       t,
		port:    listener.Addr().(*net.TCPAddr).Port,
		seen:    make(chan any, 256),
		proceed: make(chan struct{}, 8),
	}
	var open sync.WaitGroup
	var mu sync.Mutex
	var connections []net.Conn
	t.Cleanup(func() {
		// Closing ends the accept loop and every handler, which then return.
		_ = listener.Close()
		mu.Lock()
		for _, connection := range connections {
			_ = connection.Close()
		}
		mu.Unlock()
		open.Wait()
	})
	open.Add(1)
	go func() {
		defer open.Done()
		for {
			connection, err := listener.Accept()
			if err != nil {
				return
			}
			mu.Lock()
			connections = append(connections, connection)
			mu.Unlock()
			open.Add(1)
			go func() {
				defer open.Done()
				defer connection.Close()
				f.serve(connection)
			}()
		}
	}()
	return f
}

// next is what the service saw next.
func (f *httpFixture) next() any {
	f.t.Helper()
	select {
	case seen := <-f.seen:
		return seen
	case <-time.After(exposeStep):
		f.t.Fatal("the service saw nothing")
		return nil
	}
}

// nextRequest is the next request the service saw.
func (f *httpFixture) nextRequest() seenRequest {
	f.t.Helper()
	seen, ok := f.next().(seenRequest)
	if !ok {
		f.t.Fatal("the service saw something other than a request")
	}
	return seen
}

// serve answers one connection's request. An error of a connection the scenario
// ended is the connection's own end; a scenario that needs an answer fails on
// the answer it does not get.
func (f *httpFixture) serve(connection net.Conn) {
	reader := bufio.NewReader(connection)
	h, err := readHead(reader)
	if err != nil {
		return
	}
	parts := strings.Split(h.line, " ")
	if len(parts) < 2 {
		return // A malformed request has no route to serve.
	}
	target := parts[1]
	target, _, _ = strings.Cut(target, "?")
	switch target {
	case "/headers":
		f.seen <- seenRequest{head: h}
		_, _ = io.WriteString(connection, "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nX-Service-Header: Yes\r\n"+
			"Set-Cookie: first=1; Path=/\r\nset-cookie: second=2; HttpOnly\r\nContent-Length: 5\r\n"+
			"Connection: close\r\n\r\nhello")
	case "/upload":
		body, err := readChunked(reader)
		if err != nil {
			return
		}
		f.seen <- seenRequest{head: h, body: body}
		_, _ = io.WriteString(connection, "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\n\r\n")
		_ = writeChunked(connection, backendtest.Pattern(exposeBodyBytes, 7))
	case "/events":
		// Many servers abort an answer still streaming once the client's side of
		// the connection ends, so it must end last.
		ended := make(chan struct{})
		go func() {
			defer close(ended)
			// Its end, a failure or a stray byte all end the watch.
			_, _ = reader.ReadByte()
		}()
		_, _ = io.WriteString(connection, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nTransfer-Encoding: chunked\r\n\r\n")
		_, _ = io.WriteString(connection, asChunk("event: tick\ndata: 1\n\n"))
		<-f.proceed
		open := true
		select {
		case <-ended:
			open = false
		default:
		}
		_, _ = io.WriteString(connection, asChunk("event: tick\ndata: 2\n\n"))
		_, _ = io.WriteString(connection, "0\r\n\r\n")
		after := false
		select {
		case <-ended:
			after = true
		case <-time.After(exposeStep):
		}
		f.seen <- seenEventStream{openWhileStreaming: open, endedAfterAnswer: after}
	case "/hello":
		_, _ = io.WriteString(connection, "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello")
	case "/hold":
		_, _ = io.WriteString(connection, "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\n\r\n")
		_, _ = io.WriteString(connection, asChunk("held\n"))
		// Its end, a failure or a stray byte all end the hold; each proceed sends
		// one more chunk meanwhile.
		ended := make(chan struct{})
		go func() {
			defer close(ended)
			_, _ = reader.ReadByte()
		}()
	hold:
		for {
			select {
			case <-ended:
				break hold
			case <-f.proceed:
				_, _ = io.WriteString(connection, asChunk("still\n"))
			}
		}
		f.seen <- seenReleased{}
	case "/refuse":
		f.seen <- seenRequest{head: h}
		_, _ = io.WriteString(connection, "HTTP/1.1 426 Upgrade Required\r\nContent-Type: text/plain\r\nContent-Length: 17\r\n"+
			"Connection: close\r\n\r\nno upgrades here\n")
	}
}

// A visitor is a connection to the backend in raw bytes.
type visitor struct {
	t      testing.TB
	conn   net.Conn
	reader *bufio.Reader
}

func visit(t testing.TB, b *backendtest.Backend) *visitor {
	t.Helper()
	conn, err := net.Dial("tcp", fmt.Sprintf("127.0.0.1:%d", b.Port))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = conn.Close() })
	return &visitor{t: t, conn: conn, reader: bufio.NewReader(conn)}
}

func (v *visitor) write(text string) {
	v.t.Helper()
	if _, err := io.WriteString(v.conn, text); err != nil {
		v.t.Fatal(err)
	}
}

func (v *visitor) head() head {
	v.t.Helper()
	v.deadline()
	h, err := readHead(v.reader)
	if err != nil {
		v.t.Fatal(err)
	}
	return h
}

func (v *visitor) deadline() {
	_ = v.conn.SetReadDeadline(time.Now().Add(exposeStep))
}

func (v *visitor) readN(count int) []byte {
	v.t.Helper()
	v.deadline()
	data := make([]byte, count)
	if _, err := io.ReadFull(v.reader, data); err != nil {
		v.t.Fatal(err)
	}
	return data
}

func (v *visitor) chunksUntil(end string) string {
	v.t.Helper()
	v.deadline()
	text, err := readChunksUntil(v.reader, end)
	if err != nil {
		v.t.Fatalf("%v; the text was %q", err, text)
	}
	return text
}

// ended waits until the backend closed the connection, with its answer cut
// short.
func (v *visitor) ended() {
	v.t.Helper()
	v.deadline()
	_, err := io.Copy(io.Discard, v.reader)
	var timeout net.Error
	if errors.As(err, &timeout) && timeout.Timeout() {
		v.t.Fatal("the visitor's connection stays open")
	}
}

// hold is a visitor holding the fixture's /hold answer open, its first chunk
// read.
func hold(t testing.TB, b *backendtest.Backend, host string) *visitor {
	t.Helper()
	v := visit(t, b)
	v.write("GET /hold HTTP/1.1\r\nHost: " + host + "\r\n\r\n")
	if answer := v.head(); answer.line != "HTTP/1.1 200 OK" {
		t.Fatalf("the hold is answered %q", answer.line)
	}
	if first := v.chunksUntil("held\n"); first != "held\n" {
		t.Fatalf("the hold began with %q", first)
	}
	return v
}

// fetch is a visitor's request for path of the expose at host, answered whole:
// its status and its body.
func fetch(t testing.TB, b *backendtest.Backend, host, path string) (int, string) {
	t.Helper()
	v := visit(t, b)
	v.write("GET " + path + " HTTP/1.1\r\nHost: " + host + "\r\nConnection: close\r\n\r\n")
	answer := v.head()
	status, err := strconv.Atoi(scenarioItem(t, strings.Split(answer.line, " "), 1))
	if err != nil {
		t.Fatalf("the status line %q", answer.line)
	}
	v.deadline()
	body, err := io.ReadAll(v.reader)
	if err != nil {
		t.Fatal(err)
	}
	return status, string(body)
}

// exposeOn makes an expose of the fixture's port on the device, as the page
// does, and answers it.
func exposeOn(t testing.TB, b *backendtest.Backend, session *backendtest.Session, device string, port int) map[string]any {
	t.Helper()
	created := b.Post("/api/exposes", session, backendtest.Map{"deviceId": device, "address": strconv.Itoa(port)})
	created.Expect(http.StatusCreated)
	exposed, _ := created.At("expose").(map[string]any)
	return exposed
}

// hostOf is the Host a visitor of the expose sends: its URL's host and port.
func hostOf(t testing.TB, exposed map[string]any) string {
	t.Helper()
	text, _ := exposed["url"].(string)
	parsed, err := url.Parse(text)
	if err != nil || parsed.Host == "" {
		t.Fatalf("the expose's URL %q", text)
	}
	return parsed.Host
}

// exposesOf is the session's exposes, as GET /api/exposes lists them.
func exposesOf(b *backendtest.Backend, session *backendtest.Session) []any {
	listed, _ := b.Get("/api/exposes", session).Expect(http.StatusOK).At("exposes").([]any)
	return listed
}

func instant(t testing.TB, value any) time.Time {
	t.Helper()
	text, _ := value.(string)
	parsed, err := time.Parse(time.RFC3339Nano, text)
	if err != nil {
		t.Fatalf("%v is not a time: %v", value, err)
	}
	return parsed
}

func lines(name, value string) [][2]string {
	return [][2]string{{name, value}}
}

func assertLines(t testing.TB, h head, name string, want ...[2]string) {
	t.Helper()
	got := h.lines(name)
	if len(got) != len(want) {
		t.Fatalf("%s: %v, want %v (headers %v)", name, got, want, h.headers)
	}
	for index := range want {
		if got[index] != want[index] {
			t.Fatalf("%s: %v, want %v (headers %v)", name, got, want, h.headers)
		}
	}
}

func assertValues(t testing.TB, h head, name string, want ...string) {
	t.Helper()
	got := h.values(name)
	if strings.Join(got, "\n") != strings.Join(want, "\n") {
		t.Fatalf("%s: %v, want %v (headers %v)", name, got, want, h.headers)
	}
}

// Cost: one backend and a real runner, and 16 MiB across the relay, about a
// second.
func TestARelayedRequestReachesTheServiceAsSentAndItsAnswerComesBackAsSent(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t, backendtest.WithExposeDomain(exposeDomain)).StartSetUp()
	laptop := b.Pair(master, "laptop")
	fixture := startHTTPFixture(t)
	host := hostOf(t, exposeOn(t, b, master, laptop.ID(), fixture.port))

	// Header names keep their case both ways; the relay rewrites Host, adds the
	// forwarded headers and asks the service to close its connection; what
	// concerns one connection only stays behind; a repeated header keeps its
	// separate lines in order.
	v := visit(t, b)
	v.write("GET /headers?q=1 HTTP/1.1\r\nHost: " + host + "\r\nX-Custom-Header: One\r\nx-lower-case: two\r\n" +
		"X-UPPER-CASE: THREE\r\nCookie: a=1; b=2\r\nConnection: keep-alive, X-Hop\r\nKeep-Alive: timeout=5\r\n" +
		"X-Hop: gone\r\nX-Custom-Header: Four\r\n\r\n")
	answer := v.head()
	seen := fixture.nextRequest().head
	if seen.line != "GET /headers?q=1 HTTP/1.1" {
		t.Fatalf("the service saw %q", seen.line)
	}
	assertLines(t, seen, "Host", lines("Host", fmt.Sprintf("127.0.0.1:%d", fixture.port))...)
	assertLines(t, seen, "x-lower-case", lines("x-lower-case", "two")...)
	assertLines(t, seen, "X-UPPER-CASE", lines("X-UPPER-CASE", "THREE")...)
	assertLines(t, seen, "Cookie", lines("Cookie", "a=1; b=2")...)
	assertLines(t, seen, "x-custom-header", [2]string{"X-Custom-Header", "One"}, [2]string{"X-Custom-Header", "Four"})
	assertValues(t, seen, "connection", "close")
	assertValues(t, seen, "x-forwarded-for", "127.0.0.1")
	assertValues(t, seen, "x-forwarded-host", host)
	assertValues(t, seen, "x-forwarded-proto", "http")
	if len(seen.headers) != 10 {
		t.Fatalf("nothing else reaches the service: %v", seen.headers)
	}
	if answer.line != "HTTP/1.1 200 OK" {
		t.Fatalf("the answer is %q", answer.line)
	}
	assertLines(t, answer, "Content-Type", lines("Content-Type", "text/plain")...)
	assertLines(t, answer, "X-Service-Header", lines("X-Service-Header", "Yes")...)
	assertLines(t, answer, "Content-Length", lines("Content-Length", "5")...)
	assertLines(t, answer, "set-cookie", [2]string{"Set-Cookie", "first=1; Path=/"}, [2]string{"set-cookie", "second=2; HttpOnly"})
	assertLines(t, answer, "connection")
	if body := v.readN(5); string(body) != "hello" {
		t.Fatalf("the body is %q", body)
	}

	// A chunked request body of 8 MiB reaches the service whole, and so does its
	// chunked answer of 8 MiB the visitor.
	v = visit(t, b)
	v.write("POST /upload HTTP/1.1\r\nHost: " + host + "\r\nContent-Type: application/octet-stream\r\n" +
		"Transfer-Encoding: chunked\r\n\r\n")
	if err := writeChunked(v.conn, backendtest.Pattern(exposeBodyBytes, 3)); err != nil {
		t.Fatal(err)
	}
	answer = v.head()
	if answer.line != "HTTP/1.1 200 OK" {
		t.Fatalf("the answer is %q", answer.line)
	}
	assertValues(t, answer, "transfer-encoding", "chunked")
	v.deadline()
	received, err := readChunked(v.reader)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(received, backendtest.Pattern(exposeBodyBytes, 7)) {
		t.Fatal("the answer arrived changed")
	}
	uploaded := fixture.nextRequest()
	if uploaded.head.line != "POST /upload HTTP/1.1" {
		t.Fatalf("the service saw %q", uploaded.head.line)
	}
	assertValues(t, uploaded.head, "transfer-encoding", "chunked")
	if !bytes.Equal(uploaded.body, backendtest.Pattern(exposeBodyBytes, 3)) {
		t.Fatal("the request body arrived changed")
	}

	// An event stream reaches the visitor event by event: the service sends the
	// second event only once the visitor read the first. The stream's input ends
	// only once the answer is complete.
	v = visit(t, b)
	v.write("GET /events HTTP/1.1\r\nHost: " + host + "\r\nAccept: text/event-stream\r\n\r\n")
	answer = v.head()
	if answer.line != "HTTP/1.1 200 OK" {
		t.Fatalf("the answer is %q", answer.line)
	}
	assertLines(t, answer, "Content-Type", lines("Content-Type", "text/event-stream")...)
	if first := v.chunksUntil("data: 1\n\n"); first != "event: tick\ndata: 1\n\n" {
		t.Fatalf("the first event is %q", first)
	}
	fixture.proceed <- struct{}{}
	if second := v.chunksUntil("data: 2\n\n"); second != "event: tick\ndata: 2\n\n" {
		t.Fatalf("the second event is %q", second)
	}
	if _, more, err := nextChunk(v.reader); more || err != nil {
		t.Fatalf("the stream goes on after its last event: %v %v", more, err)
	}
	if stream := fixture.next(); stream != (seenEventStream{openWhileStreaming: true, endedAfterAnswer: true}) {
		t.Fatalf("the service saw %+v", stream)
	}

	// A refused upgrade reaches the visitor as the service answered it, after the
	// service saw the handshake as the visitor sent it.
	v = visit(t, b)
	v.write("GET /refuse HTTP/1.1\r\nHost: " + host + "\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
		"Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n")
	answer = v.head()
	if answer.line != "HTTP/1.1 426 Upgrade Required" {
		t.Fatalf("the answer is %q", answer.line)
	}
	assertLines(t, answer, "Content-Length", lines("Content-Length", "17")...)
	if body := v.readN(17); string(body) != "no upgrades here\n" {
		t.Fatalf("the body is %q", body)
	}
	handshake := fixture.nextRequest().head
	assertLines(t, handshake, "Upgrade", lines("Upgrade", "websocket")...)
	assertLines(t, handshake, "Connection", lines("Connection", "Upgrade")...)
	assertLines(t, handshake, "Sec-WebSocket-Key", lines("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")...)
	assertLines(t, handshake, "Sec-WebSocket-Version", lines("Sec-WebSocket-Version", "13")...)
	b.Stop()
}

type (
	// wsHandshake is what the WebSocket fixture saw of a handshake.
	wsHandshake struct{ host, key string }
	// wsClosed is a close frame the visitor sent.
	wsClosed struct {
		code   websocket.StatusCode
		reason string
	}
)

// A webSocketFixture is a WebSocket service on this machine: it echoes text and
// binary messages, answers pings, and closes with 4002 when the visitor sends
// the text "close".
type webSocketFixture struct {
	t    testing.TB
	port int
	seen chan any
}

func startWebSocketFixture(t testing.TB) *webSocketFixture {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	f := &webSocketFixture{t: t, port: listener.Addr().(*net.TCPAddr).Port, seen: make(chan any, 32)}
	server := &http.Server{Handler: http.HandlerFunc(f.serve), ReadHeaderTimeout: exposeStep}
	served := make(chan struct{})
	go func() {
		defer close(served)
		// Serve answers an error once the scenario closes the server.
		_ = server.Serve(listener)
	}()
	t.Cleanup(func() {
		_ = server.Close()
		<-served
	})
	return f
}

func (f *webSocketFixture) next() any {
	f.t.Helper()
	select {
	case seen := <-f.seen:
		return seen
	case <-time.After(exposeStep):
		f.t.Fatal("the service saw nothing")
		return nil
	}
}

func (f *webSocketFixture) serve(w http.ResponseWriter, r *http.Request) {
	conn, err := websocket.Accept(w, r, nil)
	if err != nil {
		return
	}
	defer conn.CloseNow()
	f.seen <- wsHandshake{host: r.Host, key: r.Header.Get("Sec-WebSocket-Key")}
	for {
		kind, data, err := conn.Read(r.Context())
		if err != nil {
			var closed websocket.CloseError
			if errors.As(err, &closed) {
				f.seen <- wsClosed{code: closed.Code, reason: closed.Reason}
			}
			return
		}
		if kind == websocket.MessageText && string(data) == "close" {
			// The visitor's answer to this close ends the exchange; its error
			// says only that the handshake was not clean.
			_ = conn.Close(4002, "service done")
			return
		}
		if err := conn.Write(r.Context(), kind, data); err != nil {
			return
		}
	}
}

// dialSocket is a WebSocket of a visitor to the expose at host: its connection
// goes to the backend, whatever host names.
func dialSocket(t testing.TB, b *backendtest.Backend, host string) (*websocket.Conn, *http.Response) {
	t.Helper()
	client := &http.Client{Transport: &http.Transport{
		DialContext: func(ctx context.Context, network, _ string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, network, fmt.Sprintf("127.0.0.1:%d", b.Port))
		},
	}}
	ctx, cancel := context.WithTimeout(context.Background(), exposeStep)
	defer cancel()
	conn, response, err := websocket.Dial(ctx, "ws://"+host+"/socket", &websocket.DialOptions{HTTPClient: client})
	if err != nil {
		t.Fatalf("the visitor's handshake: %v", err)
	}
	t.Cleanup(func() { _ = conn.CloseNow() })
	return conn, response
}

// socketMessage is a message a visitor's WebSocket read.
type socketMessage struct {
	kind websocket.MessageType
	data []byte
	err  error
}

// readMessages reads the connection in the background, so that pings are
// answered while the scenario waits; the last message carries the error that
// ended it.
func readMessages(conn *websocket.Conn) <-chan socketMessage {
	messages := make(chan socketMessage, 16)
	go func() {
		defer close(messages)
		for {
			kind, data, err := conn.Read(context.Background())
			messages <- socketMessage{kind: kind, data: data, err: err}
			if err != nil {
				return
			}
		}
	}()
	return messages
}

func nextMessage(t testing.TB, messages <-chan socketMessage) socketMessage {
	t.Helper()
	select {
	case message, open := <-messages:
		if !open {
			t.Fatal("the visitor's socket ended")
		}
		return message
	case <-time.After(exposeStep):
		t.Fatal("the visitor's socket got nothing")
		return socketMessage{}
	}
}

// Cost: one backend and a real runner, about a second.
func TestAWebSocketThroughTheRelayCarriesMessagesAndCloseCodesBothWays(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t, backendtest.WithExposeDomain(exposeDomain)).StartSetUp()
	laptop := b.Pair(master, "laptop")
	fixture := startWebSocketFixture(t)
	host := hostOf(t, exposeOn(t, b, master, laptop.ID(), fixture.port))

	// The visitor's handshake reaches the service with its key, and the service's
	// switch reaches the visitor.
	conn, switched := dialSocket(t, b, host)
	if switched.StatusCode != http.StatusSwitchingProtocols || switched.Header.Get("Sec-WebSocket-Accept") == "" {
		t.Fatalf("the switch is %d %v", switched.StatusCode, switched.Header)
	}
	handshake, ok := fixture.next().(wsHandshake)
	if !ok || handshake.host != fmt.Sprintf("127.0.0.1:%d", fixture.port) || handshake.key == "" {
		t.Fatalf("the service saw %+v", handshake)
	}
	messages := readMessages(conn)
	ctx, cancel := context.WithTimeout(context.Background(), exposeStep)
	defer cancel()
	// Text and binary come back unchanged, and a ping is answered by the service's
	// pong.
	for _, sent := range []socketMessage{
		{kind: websocket.MessageText, data: []byte("hello")},
		{kind: websocket.MessageBinary, data: []byte{0, 1, 2, 255}},
	} {
		if err := conn.Write(ctx, sent.kind, sent.data); err != nil {
			t.Fatal(err)
		}
		echoed := nextMessage(t, messages)
		if echoed.err != nil || echoed.kind != sent.kind || !bytes.Equal(echoed.data, sent.data) {
			t.Fatalf("%v %q came back as %+v", sent.kind, sent.data, echoed)
		}
	}
	if err := conn.Ping(ctx); err != nil {
		t.Fatalf("the service does not answer a ping: %v", err)
	}
	// The visitor's close code and reason reach the service, whose answer reaches
	// the visitor.
	// The close call's own result says only whether its handshake was clean; the
	// answer is read from the socket.
	_ = conn.Close(4001, "visitor done")
	if closed := fixture.next(); closed != (wsClosed{code: 4001, reason: "visitor done"}) {
		t.Fatalf("the service saw %+v", closed)
	}
	for message := range messages {
		if message.err == nil {
			continue
		}
		var answer websocket.CloseError
		if !errors.As(message.err, &answer) || answer.Code != 4001 || answer.Reason != "visitor done" {
			t.Fatalf("the visitor's socket ended with %v", message.err)
		}
	}

	// The service's close code and reason reach the visitor.
	conn, _ = dialSocket(t, b, host)
	if _, ok := fixture.next().(wsHandshake); !ok {
		t.Fatal("the service saw no handshake")
	}
	messages = readMessages(conn)
	if err := conn.Write(ctx, websocket.MessageText, []byte("close")); err != nil {
		t.Fatal(err)
	}
	ended := nextMessage(t, messages)
	var answer websocket.CloseError
	if !errors.As(ended.err, &answer) || answer.Code != 4002 || answer.Reason != "service done" {
		t.Fatalf("the visitor's socket ended with %v", ended.err)
	}
	b.Stop()
}

// The Rust test also renews the expose before its hour, lets it expire on the
// clock and finds its record destroyed; those parts need the manual clock and
// are not here (G6b).
//
// Cost: one backend and a real runner, about a second.
func TestAnExposeAnswersAnyoneAndOnlyItsOwnerListsRenewsOrRemovesIt(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t, backendtest.WithExposeDomain(exposeDomain)).StartSetUp()
	laptop := b.Pair(master, "laptop")
	fixture := startHTTPFixture(t)

	// Made on a connected device of the caller's for an hour, under a URL with the
	// backend's scheme and port; the snapshot shows it with the domain, and a
	// visitor without a session reaches the service.
	exposed := exposeOn(t, b, master, laptop.ID(), fixture.port)
	id, _ := exposed["id"].(string)
	if want := fmt.Sprintf("http://%s.%s:%d/", id, exposeDomain, b.Port); exposed["url"] != want {
		t.Fatalf("the URL is %v, want %s", exposed["url"], want)
	}
	if want := fmt.Sprintf("127.0.0.1:%d", fixture.port); exposed["address"] != want {
		t.Fatalf("the address is %v, want %s", exposed["address"], want)
	}
	lifetime := instant(t, exposed["expiresAt"]).Sub(instant(t, exposed["createdAt"]))
	if lifetime != time.Hour {
		t.Fatalf("the expose lives %v", lifetime)
	}
	state := b.Sync(master).Snapshot()
	backendtest.AssertJSON(t, state["exposes"], []any{exposed})
	if state["exposeDomain"] != exposeDomain {
		t.Fatalf("the snapshot's domain is %v", state["exposeDomain"])
	}
	backendtest.AssertJSON(t, exposesOf(b, master), []any{exposed})
	host := hostOf(t, exposed)
	if status, body := fetch(t, b, host, "/hello"); status != 200 || body != "hello" {
		t.Fatalf("the visitor got %d %q", status, body)
	}

	// Another user sees none of it and can change none of it, nor expose a service
	// on the device.
	other := b.CreateUser(master, "other@example.test", "other-pass-1", "user")
	if listed := exposesOf(b, other); len(listed) != 0 {
		t.Fatalf("another user lists %v", listed)
	}
	wantRefusal(t, b.Post("/api/exposes/"+id+"/renew", other, backendtest.Map{}), http.StatusNotFound, "expose_not_found", "another user's renewal")
	wantRefusal(t, b.Delete("/api/exposes/"+id, other), http.StatusNotFound, "expose_not_found", "another user's removal")
	wantRefusal(t, b.Post("/api/exposes", other, backendtest.Map{"deviceId": laptop.ID(), "address": "8080"}),
		http.StatusNotFound, "device_not_found", "an expose on another user's device")
	wantRefusal(t, b.Post("/api/exposes", master, backendtest.Map{"deviceId": laptop.ID(), "address": "localhost"}),
		http.StatusBadRequest, "invalid_body", "an address without a port")

	// Removed, it is gone at once.
	removed := exposeOn(t, b, master, laptop.ID(), fixture.port)
	removedID, _ := removed["id"].(string)
	b.Delete("/api/exposes/"+removedID, master).Expect(http.StatusNoContent)
	if status, _ := fetch(t, b, hostOf(t, removed), "/hello"); status != 404 {
		t.Fatalf("a removed expose answers %d", status)
	}
	backendtest.AssertJSON(t, exposesOf(b, master), []any{exposed})
	wantRefusal(t, b.Delete("/api/exposes/"+removedID, master), http.StatusNotFound, "expose_not_found", "a second removal")
	b.Stop()
}

// The Rust test also holds a connection across the expose's expiry and finds
// its record destroyed; that part needs the manual clock and is not here (G6b).
//
// Cost: one backend and a real runner that stops and starts again, about half
// a second.
func TestOpenConnectionsEndWithTheirExposeAndAnOfflineDeviceKeepsItsExposes(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t, backendtest.WithExposeDomain(exposeDomain)).StartSetUp()
	laptop := b.Pair(master, "laptop")
	fixture := startHTTPFixture(t)

	// Removed while a visitor holds an answer open: the visitor's connection
	// closes, and the runner closes its socket to the service.
	removed := exposeOn(t, b, master, laptop.ID(), fixture.port)
	held := hold(t, b, hostOf(t, removed))
	b.Delete("/api/exposes/"+removed["id"].(string), master).Expect(http.StatusNoContent)
	held.ended()
	if released := fixture.next(); released != (seenReleased{}) {
		t.Fatalf("the service saw %+v", released)
	}

	// A device whose runner is gone keeps its exposes, which answer 502 until the
	// runner is back; nothing new is exposed on it meanwhile.
	kept := exposeOn(t, b, master, laptop.ID(), fixture.port)
	laptop.Runner.Kill()
	b.UntilOnline(master, laptop.ID(), false)
	status, page := fetch(t, b, hostOf(t, kept), "/hello")
	if status != 502 || !strings.Contains(page, "device_offline") {
		t.Fatalf("an offline device's expose answers %d %q", status, page)
	}
	backendtest.AssertJSON(t, exposesOf(b, master), []any{kept})
	wantRefusal(t, b.Post("/api/exposes", master, backendtest.Map{"deviceId": laptop.ID(), "address": strconv.Itoa(fixture.port)}),
		http.StatusConflict, "device_offline", "an expose on an offline device")
	if err := laptop.Runner.StartAgain(); err != nil {
		t.Fatal(err)
	}
	b.UntilOnline(master, laptop.ID(), true)
	if status, body := fetch(t, b, hostOf(t, kept), "/hello"); status != 200 || body != "hello" {
		t.Fatalf("the returned device answers %d %q", status, body)
	}

	// Revoked, the device's exposes end with their connections.
	held = hold(t, b, hostOf(t, kept))
	b.Delete("/api/devices/"+laptop.ID(), master).Expect(http.StatusNoContent)
	held.ended()
	if released := fixture.next(); released != (seenReleased{}) {
		t.Fatalf("the service saw %+v", released)
	}
	if listed := exposesOf(b, master); len(listed) != 0 {
		t.Fatalf("a revoked device's exposes are listed: %v", listed)
	}
	if status, _ := fetch(t, b, hostOf(t, kept), "/hello"); status != 404 {
		t.Fatalf("a revoked device's expose answers %d", status)
	}
	b.Stop()
}

// Cost: one backend, a real runner and 65 connections, about a second.
func TestAnExposeShedsItsSixtyFifthConnectionAndAClosedOneMakesRoom(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t, backendtest.WithExposeDomain(exposeDomain)).StartSetUp()
	laptop := b.Pair(master, "laptop")
	fixture := startHTTPFixture(t)
	host := hostOf(t, exposeOn(t, b, master, laptop.ID(), fixture.port))
	var held []*visitor
	for range 64 {
		held = append(held, hold(t, b, host))
	}
	status, page := fetch(t, b, host, "/hello")
	if status != 503 || !strings.Contains(page, "connection limit") {
		t.Fatalf("the 65th connection is answered %d %q", status, page)
	}
	// The visitor goes away, and once the relay saw it go its place is free.
	_ = held[63].conn.Close()
	backendtest.Eventually(t, "a closed connection makes room", func() bool {
		status, _ := fetch(t, b, host, "/hello")
		return status == 200
	})
	b.Stop()
}

// Cost: one backend and a real runner; the relay's idle limit of one second
// passes in real time, about 1.3 seconds.
func TestARelayedConnectionNothingMovesOnClosesAfterTheIdleLimit(t *testing.T) {
	t.Parallel()
	h := backendtest.New(t, backendtest.WithExposeDomain(exposeDomain))
	h.Exposes().IdleMs = backendtest.Ptr(uint64(1000))
	b, master := h.StartSetUp()
	laptop := b.Pair(master, "laptop")
	fixture := startHTTPFixture(t)
	host := hostOf(t, exposeOn(t, b, master, laptop.ID(), fixture.port))
	// The visitor read the first bytes of an answer that goes on, then neither
	// side sends another.
	held := hold(t, b, host)
	quiet := time.Now()
	held.ended()
	if elapsed := time.Since(quiet); elapsed < 900*time.Millisecond {
		t.Fatalf("closed after %v", elapsed)
	}
	if released := fixture.next(); released != (seenReleased{}) {
		t.Fatalf("the service saw %+v", released)
	}
	b.Stop()
}

// Cost: one backend and a real runner, and three turns of the builtin package's
// `demi host expose`, several seconds.
func TestTheAgentExposesAServiceAndListsRenewsAndRemovesExposesWithDemiHostExpose(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	b, master := backendtest.New(t, backendtest.WithBuiltin(), backendtest.WithExposeDomain(exposeDomain)).StartSetUp()
	fixture := startHTTPFixture(t)
	provider := b.Anthropic(master, vendor, "/work")
	b.CreateConversation(master, exposeAgentConv)
	onDeviceConversation(t, b, master, exposeAgentConv)
	work := b.Open(master, vendor, exposeAgentConv, provider, "/work")

	// `add` exposes a service on the conversation's main Host; each of the user's
	// exposes takes the user's next number.
	add := fmt.Sprintf("demi host expose add %d", fixture.port)
	added := work.Turn(backendtest.ShellCall("t1", add+" && "+add, 20*time.Second), backendtest.Say("exposed"))
	exposed := exposesOf(b, master)
	numbered := func(number int) map[string]any {
		for _, candidate := range exposed {
			if value, _ := backendtest.At(candidate, "number").(float64); int(value) == number {
				return candidate.(map[string]any)
			}
		}
		t.Fatalf("no expose %d: %v", number, exposed)
		return nil
	}
	first, second := numbered(1), numbered(2)
	for _, exposedOne := range []map[string]any{first, second} {
		printed := fmt.Sprintf("Exposed 127.0.0.1:%d on laptop as %v\nExpires in 60 minutes (expose %v).\n",
			fixture.port, exposedOne["url"], exposedOne["number"])
		if !strings.Contains(backendtest.ShownOutput(added.Result(t, 0)), printed) {
			t.Fatalf("%q is not in %s", printed, added.Result(t, 0))
		}
	}
	if status, body := fetch(t, b, hostOf(t, first), "/hello"); status != 200 || body != "hello" {
		t.Fatalf("the agent's expose answers %d %q", status, body)
	}

	// `list` shows every expose of the user under a header, or as JSON.
	listed := work.Turn(backendtest.ShellCall("t2", "demi host expose list && demi host expose list --json", 20*time.Second), backendtest.Say("listed"))
	address, _ := first["address"].(string)
	header := fmt.Sprintf("Expose  Device  %-*s  Expires  URL\n", len(address), "Address")
	if !strings.Contains(listed.Result(t, 0), header) {
		t.Fatalf("no header %q in %s", header, listed.Result(t, 0))
	}
	for _, exposedOne := range []map[string]any{first, second} {
		row := fmt.Sprintf("%-6v  laptop  %s  60 min   %v\n", exposedOne["number"], address, exposedOne["url"])
		if !strings.Contains(listed.Result(t, 0), row) {
			t.Fatalf("no row %q in %s", row, listed.Result(t, 0))
		}
	}
	var printed []any
	for line := range strings.SplitSeq(listed.Result(t, 0), "\n") {
		if strings.HasPrefix(line, `{"exposes":`) {
			printed, _ = backendtest.At(backendtest.Decode(t, []byte(line)), "exposes").([]any)
		}
	}
	backendtest.AssertJSON(t, printed, exposed)

	// Another user's exposes are numbered apart: theirs is 1 too.
	other := b.CreateUser(master, "other@example.test", "other-pass-1", "user")
	desk := b.Pair(other, "desk")
	theirs := exposeOn(t, b, other, desk.ID(), fixture.port)
	if theirs["number"] != float64(1) {
		t.Fatalf("another user's expose is number %v", theirs["number"])
	}

	// `renew` and `remove` take the number; a number the user has no live expose
	// of is not found, though another user's expose has it; the next add takes a
	// number never given; a host the conversation does not reach is refused.
	changes := "demi host expose renew 1 && demi host expose remove 1 && demi host expose renew 1; echo exit=$?; " +
		add + "; demi host expose add 8080 --host nope; echo exit=$?"
	changed := work.Turn(backendtest.ShellCall("t3", changes, 20*time.Second), backendtest.Say("changed"))
	received := changed.Result(t, 0)
	for _, expected := range []string{
		"Expose 1 expires in 60 minutes.\n",
		"Removed expose 1; its URL no longer works.\n",
		"expose renew: No expose 1 (expose_not_found)\n",
		"Expires in 60 minutes (expose 3).\n",
		"host nope is not reachable from this conversation",
	} {
		if !strings.Contains(received, expected) {
			t.Fatalf("%q is not in %s", expected, received)
		}
	}
	if count := strings.Count(received, "exit=1"); count != 2 {
		t.Fatalf("%d commands failed, want 2:\n%s", count, received)
	}
	if status, _ := fetch(t, b, hostOf(t, first), "/hello"); status != 404 {
		t.Fatalf("a removed expose answers %d", status)
	}
	backendtest.AssertJSON(t, exposesOf(b, other), []any{theirs})
	b.Stop()
}

// Cost: one backend, about a second.
func TestWithoutAnExposeDomainExposesAreUnavailable(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t).StartSetUp()
	laptop := b.Pair(master, "laptop")
	wantRefusal(t, b.Post("/api/exposes", master, backendtest.Map{"deviceId": laptop.ID(), "address": "1234"}),
		http.StatusConflict, "expose_unavailable", "an expose without a domain")
	state := b.Sync(master).Snapshot()
	backendtest.AssertJSON(t, []any{state["exposes"], state["exposeDomain"]}, []any{[]any{}, nil})
	if listed := exposesOf(b, master); len(listed) != 0 {
		t.Fatalf("exposes are listed: %v", listed)
	}
	b.Stop()
}

// Cost: one backend and a real runner, about a second.
func TestAnExposeIsRenewedBeforeItsHourAndAnswersUnknownASecondAfterItsExpiry(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t, backendtest.WithExposeDomain(exposeDomain)).StartSetUp()
	laptop := b.Pair(master, "laptop")
	fixture := startHTTPFixture(t)
	exposed := exposeOn(t, b, master, laptop.ID(), fixture.port)
	id, _ := exposed["id"].(string)
	host := hostOf(t, exposed)

	// Renewed before its hour, it lives an hour from the renewal.
	now := b.Control.AdvanceClock(50 * time.Minute)
	renewed := b.Post("/api/exposes/"+id+"/renew", master, backendtest.Map{}).Expect(http.StatusOK)
	if got := instant(t, renewed.At("expose.expiresAt")); !got.Equal(now.Add(time.Hour)) {
		t.Fatalf("the renewed expose ends at %v, not %v", got, now.Add(time.Hour))
	}
	b.Control.AdvanceClock(50 * time.Minute)
	if status, _ := fetch(t, b, host, "/hello"); status != 200 {
		t.Fatalf("the renewed expose answers %d", status)
	}

	// A second after its expiry the URL answers as unknown, and the visit
	// destroyed the record.
	b.Control.AdvanceClock(10*time.Minute + time.Second)
	status, page := fetch(t, b, host, "/hello")
	if status != 404 || !strings.Contains(page, "does not exist") {
		t.Fatalf("an expired expose answers %d %q", status, page)
	}
	if listed := exposesOf(b, master); len(listed) != 0 {
		t.Fatalf("an expired expose is listed: %v", listed)
	}
	wantRefusal(t, b.Post("/api/exposes/"+id+"/renew", master, backendtest.Map{}), http.StatusNotFound, "expose_not_found", "the renewal of an expired expose")
	b.Stop()
}

// Cost: one backend and a real runner, about a second.
func TestAConnectionHeldAcrossItsExposesExpiryEndsAtTheExpiry(t *testing.T) {
	t.Parallel()
	b, master := backendtest.New(t, backendtest.WithExposeDomain(exposeDomain)).StartSetUp()
	laptop := b.Pair(master, "laptop")
	fixture := startHTTPFixture(t)
	expiring := exposeOn(t, b, master, laptop.ID(), fixture.port)
	b.Control.AdvanceClock(time.Hour - time.Second)
	held := hold(t, b, hostOf(t, expiring))
	b.Control.AdvanceClock(2 * time.Second)
	held.ended()
	if released := fixture.next(); released != (seenReleased{}) {
		t.Fatalf("the service saw %+v", released)
	}
	if listed := exposesOf(b, master); len(listed) != 0 {
		t.Fatalf("an expired expose is listed: %v", listed)
	}
	b.Stop()
}

// Cost: one backend, a scripted manager and a Cloud's runner, several seconds: a
// Cloud boots and installs the builtin package, checkpoints, and then idles for a
// window that passes in real time.
func TestACloudsExposesOutliveItsCheckpointsAndEndWhenItStopsIdle(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	const window = 600 * time.Millisecond
	h := backendtest.New(t, backendtest.WithBuiltin(), backendtest.WithExposeDomain(exposeDomain))
	idleAfter(h, window)
	h.Cloud().SweepMs = backendtest.Ptr(uint64(50))
	h.Cloud().CheckpointIntervalMs = backendtest.Ptr(uint64(300))
	b, master := h.StartSetUp()
	fixture := startHTTPFixture(t)
	provider := b.Anthropic(master, vendor, "/work")
	b.CreateConversation(master, exposeAgentConv)
	// A lease of the conversation's file gate is its work: the Cloud does not idle
	// before the test has looked, and idles once it ends.
	working := b.Control.EnterGate(master.User.ID, exposeAgentConv, "demand")
	work := b.Open(master, vendor, exposeAgentConv, provider, "/work")
	work.Turn(backendtest.ShellCall("t1", "true", 20*time.Second), backendtest.Say("awake"))
	device := theCloud(t, h)
	exposed := exposeOn(t, b, master, device, fixture.port)
	host := hostOf(t, exposed)
	held := hold(t, b, host)

	// A checkpoint keeps the Cloud running, and its expose and the expose's
	// connection with it: what the service sends on the connection afterwards
	// still arrives.
	checkpoint := "checkpoint:" + device
	before := h.Manager.Count(checkpoint)
	backendtest.Eventually(t, "the Cloud checkpoints", func() bool { return h.Manager.Count(checkpoint) > before })
	if status, body := fetch(t, b, host, "/hello"); status != 200 || body != "hello" {
		t.Fatalf("the Cloud's expose answers %d %q", status, body)
	}
	fixture.proceed <- struct{}{}
	if got := held.chunksUntil("still\n"); got != "still\n" {
		t.Fatalf("the connection carries %q", got)
	}
	backendtest.AssertJSON(t, exposesOf(b, master), []any{exposed})

	// Idle, the Cloud stops a window after its work ends, and its expose ends with
	// it, before the Cloud is saved.
	rested := time.Now()
	working.Release()
	held.ended()
	if released := fixture.next(); released != (seenReleased{}) {
		t.Fatalf("the service saw %+v", released)
	}
	if listed := exposesOf(b, master); len(listed) != 0 {
		t.Fatalf("the stopped Cloud's expose is listed: %v", listed)
	}
	stopped := h.Manager.Arrival("hibernate:"+device, backendtest.Patience)
	if stopped.Before(rested.Add(window)) {
		t.Fatalf("the Cloud stopped %v after its work ended", stopped.Sub(rested))
	}
	if status, _ := fetch(t, b, host, "/hello"); status != 404 {
		t.Fatalf("a stopped Cloud's expose answers %d", status)
	}
	b.Stop()
}

// Cost: one backend, a scripted manager and a Cloud's runner booted three times,
// several seconds: the Cloud boots again after its death and after its reset, and
// installs the builtin package each time.
func TestACloudsExposesEndWhenItDiesResetsOrIsFoundStoppedAndBeforeABackendServes(t *testing.T) {
	t.Parallel()
	vendor := scripted.StartVendor(t)
	h := backendtest.New(t, backendtest.WithBuiltin(), backendtest.WithExposeDomain(exposeDomain))
	b, master := h.StartSetUp()
	fixture := startHTTPFixture(t)
	provider := b.Anthropic(master, vendor, "/work")
	b.CreateConversation(master, exposeAgentConv)
	work := b.Open(master, vendor, exposeAgentConv, provider, "/work")
	work.Turn(backendtest.ShellCall("t1", "true", 20*time.Second), backendtest.Say("awake"))
	device := theCloud(t, h)
	released := func() {
		t.Helper()
		if seen := fixture.next(); seen != (seenReleased{}) {
			t.Fatalf("the service saw %+v", seen)
		}
	}
	noExposes := func(what string) {
		backendtest.Eventually(t, what, func() bool { return len(exposesOf(b, master)) == 0 })
	}

	// The Cloud's sandbox dies.
	dying := exposeOn(t, b, master, device, fixture.port)
	held := hold(t, b, hostOf(t, dying))
	h.Manager.Kill(device)
	held.ended()
	released()
	noExposes("the dead Cloud's expose ends")

	// The next operation boots it again, and a reset stops it.
	work.Turn(backendtest.ShellCall("t2", "true", 20*time.Second), backendtest.Say("awake again"))
	resetting := exposeOn(t, b, master, device, fixture.port)
	held = hold(t, b, hostOf(t, resetting))
	resetCloud(b, master, cloudReset)
	held.ended()
	released()
	untilCloud(t, b, master, "the reset is ready", resetIs("ready"))
	if listed := exposesOf(b, master); len(listed) != 0 {
		t.Fatalf("a reset Cloud's exposes are listed: %v", listed)
	}

	// A Cloud stopped without a word keeps its expose, offline, until the next
	// operation finds it stopped and boots it again.
	found := exposeOn(t, b, master, device, fixture.port)
	h.Manager.StopQuietly(device)
	backendtest.Eventually(t, "the backend sees the runner go", func() bool { return !cloudOnline(b, master) })
	backendtest.AssertJSON(t, exposesOf(b, master), []any{found})
	if status, _ := fetch(t, b, hostOf(t, found), "/hello"); status != 502 {
		t.Fatalf("an expose of a Cloud stopped without a word answers %d", status)
	}
	work.Turn(backendtest.ShellCall("t3", "true", 20*time.Second), backendtest.Say("booted"))
	if listed := exposesOf(b, master); len(listed) != 0 {
		t.Fatalf("the found stopped Cloud's exposes are listed: %v", listed)
	}
	if status, _ := fetch(t, b, hostOf(t, found), "/hello"); status != 404 {
		t.Fatalf("an ended expose answers %d", status)
	}

	// A backend that stopped without saving its Cloud, as a crash does, leaves its
	// exposes behind; the next one ends them before it serves, since the machine
	// manager has stopped every Cloud by then.
	crashing := exposeOn(t, b, master, device, fixture.port)
	b.Kill()
	b = h.Start()
	if listed := exposesOf(b, master); len(listed) != 0 {
		t.Fatalf("the crashed backend's expose %v outlived its Cloud", crashing["id"])
	}
	b.Stop()
}
