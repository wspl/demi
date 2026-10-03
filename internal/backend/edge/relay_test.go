package edge

import (
	"bufio"
	"bytes"
	"context"
	"io"
	"net"
	"net/http"
	"strings"
	"testing"
	"testing/synctest"
	"time"
)

func TestExposeRelayPreservesMixedCaseAndRepeatedHeaders(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		visitor, edge := net.Pipe()
		service, relay := net.Pipe()
		defer func() {
			_ = visitor.Close()
			_ = edge.Close()
			_ = service.Close()
			_ = relay.Close()
		}()
		head := []byte(
			"POST /path?q=1 HTTP/1.1\r\nHost: e_x.example.test\r\n" +
				"x-MiXeD-Case: first\r\nx-MiXeD-Case: second\r\ncOnTeNt-LeNgTh: 2\r\n" +
				"Connection: x-remove\r\nx-remove: gone\r\n\r\n",
		)
		relayed := make(chan struct{})
		go func() {
			defer close(relayed)
			forwardRelay(
				t.Context(),
				edge,
				bufio.NewReader(edge),
				head,
				relayTestConn{relay},
				func() { _ = relay.SetDeadline(time.Now()) },
				"localhost:3000",
				time.Minute,
			)
		}()
		sent := make(chan error, 1)
		go func() {
			_, err := io.WriteString(visitor, "hi")
			sent <- err
		}()
		serviceInput := bufio.NewReader(service)
		incoming, err := readHead(serviceInput)
		if err != nil {
			t.Fatal(err)
		}
		text := string(incoming)
		if !strings.Contains(text, "x-MiXeD-Case: first\r\nx-MiXeD-Case: second\r\n") ||
			!strings.Contains(text, "cOnTeNt-LeNgTh: 2\r\n") ||
			strings.Contains(text, "Content-Length:") ||
			strings.Contains(text, "X-Mixed-Case") ||
			strings.Contains(text, "x-remove:") ||
			strings.Contains(text, "User-Agent:") {
			t.Fatalf("forwarded head:\n%s", text)
		}
		body := make([]byte, 2)
		if _, err := io.ReadFull(serviceInput, body); err != nil || string(body) != "hi" {
			t.Fatal(string(body), err)
		}
		if err := <-sent; err != nil {
			t.Fatal(err)
		}
		answered := make(chan error, 1)
		go func() {
			_, err := io.WriteString(service, "HTTP/1.1 200 OK\r\ncOnTeNt-LeNgTh: 2\r\nx-RePlY: kept\r\n\r\nok")
			answered <- err
		}()
		visitorInput := bufio.NewReader(visitor)
		replyHead, err := readHead(visitorInput)
		if err != nil {
			t.Fatal(err)
		}
		if !strings.Contains(string(replyHead), "x-RePlY: kept\r\n") ||
			!strings.Contains(string(replyHead), "cOnTeNt-LeNgTh: 2\r\n") {
			t.Fatalf("reply changed: %s", replyHead)
		}
		answer, err := http.ReadResponse(bufio.NewReader(io.MultiReader(bytes.NewReader(replyHead), visitorInput)), nil)
		if err != nil {
			t.Fatal(err)
		}
		body, err = io.ReadAll(answer.Body)
		_ = answer.Body.Close()
		if err != nil || string(body) != "ok" {
			t.Fatal(string(body), err)
		}
		if err := <-answered; err != nil {
			t.Fatal(err)
		}
		<-relayed
	})
}

func TestExposeUpgradeCopiesUnreadBytesBothWays(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		visitor, edge := net.Pipe()
		service, relay := net.Pipe()
		defer func() {
			_ = visitor.Close()
			_ = edge.Close()
			_ = service.Close()
			_ = relay.Close()
		}()
		head := []byte(
			"GET /socket HTTP/1.1\r\nHost: example.test\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n",
		)
		relayed := make(chan struct{})
		go func() {
			defer close(relayed)
			forwardRelay(
				t.Context(),
				edge,
				bufio.NewReader(edge),
				head,
				relayTestConn{relay},
				func() { _ = relay.SetDeadline(time.Now()) },
				"localhost:3000",
				time.Minute,
			)
		}()
		if _, err := readHead(bufio.NewReader(service)); err != nil {
			t.Fatal(err)
		}
		sent := make(chan error, 1)
		go func() {
			_, err := io.WriteString(
				service,
				"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\nfrom service",
			)
			sent <- err
		}()
		input := bufio.NewReader(visitor)
		response, err := readHead(input)
		if err != nil || !bytes.HasPrefix(response, []byte("HTTP/1.1 101")) {
			t.Fatal(string(response), err)
		}
		message := make([]byte, len("from service"))
		if _, err := io.ReadFull(input, message); err != nil || string(message) != "from service" {
			t.Fatal(string(message), err)
		}
		if err := <-sent; err != nil {
			t.Fatal(err)
		}
		go func() {
			_, err := io.WriteString(visitor, "from visitor")
			sent <- err
		}()
		if _, err := io.ReadFull(service, message); err != nil || string(message) != "from visitor" {
			t.Fatal(string(message), err)
		}
		if err := <-sent; err != nil {
			t.Fatal(err)
		}
		_ = visitor.Close()
		<-relayed
	})
}

// Real remotehost pipe ends expose ownership races that net.Pipe's concurrent
// Close hides. The upgraded visitor disconnects with a service read in flight.
// No sleeps or external services; expected cost is under one second.
func TestExposeUpgradeJoinsCopiesBeforeClosingPipeEnds(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		visitor, edge := net.Pipe()
		defer func() { _ = visitor.Close() }()
		defer func() { _ = edge.Close() }()
		toService, serviceInput := testPipe(t)
		serviceOutput, fromService := testPipe(t)
		ctx, cancel := context.WithCancel(t.Context())
		defer cancel()
		stream := &relayPipe{ctx: ctx, reader: fromService, writer: toService}
		peer := &relayPipe{ctx: ctx, reader: serviceInput, writer: serviceOutput}
		done := make(chan struct{})
		go func() {
			defer close(done)
			head := []byte("GET / HTTP/1.1\r\nHost: expose.test\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n")
			forwardRelay(ctx, edge, bufio.NewReader(edge), head, stream, cancel, "localhost:3000", time.Minute)
		}()
		defer func() {
			_ = visitor.Close()
			cancel()
			<-done
		}()
		if _, err := readHead(bufio.NewReader(peer)); err != nil {
			t.Fatal(err)
		}
		drained := make(chan struct{})
		go func() {
			defer close(drained)
			// The service answers read EOF by ending its reply, as a TCP peer does.
			_, _ = io.Copy(io.Discard, peer)
			serviceOutput.End()
		}()
		defer func() {
			cancel()
			<-drained
		}()
		if err := serviceOutput.Write(
			t.Context(),
			[]byte("HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\nready"),
		); err != nil {
			t.Fatal(err)
		}
		input := bufio.NewReader(visitor)
		if _, err := readHead(input); err != nil {
			t.Fatal(err)
		}
		data := make([]byte, 5)
		if _, err := io.ReadFull(input, data); err != nil || string(data) != "ready" {
			t.Fatal(string(data), err)
		}
		synctest.Wait()
		_ = visitor.Close()
		<-done
	})
}

// net.Pipe has no half-close. Its concurrency-safe Close ends the test peer.
type relayTestConn struct{ net.Conn }

func (c relayTestConn) CloseWrite() error { return c.Close() }
