package edge

import (
	"bufio"
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httputil"
	"strconv"
	"strings"
	"time"

	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/webapi"
)

type rawHeader struct{ name, value string }

func originalHeaders(head []byte) []rawHeader {
	lines := bytes.Split(head, []byte("\n"))
	headers := make([]rawHeader, 0, len(lines))
	for _, line := range lines[1:] {
		name, value, ok := bytes.Cut(bytes.TrimSuffix(line, []byte("\r")), []byte(":"))
		if ok {
			headers = append(headers, rawHeader{string(name), strings.TrimSpace(string(value))})
		}
	}
	return headers
}
func (e *Edge) exposeLabel(head []byte) (string, bool) {
	if e.state.Services.ExposeDomain == nil {
		return "", false
	}
	for _, header := range originalHeaders(head) {
		if strings.EqualFold(header.name, "host") {
			host := header.value
			if hostname, _, err := net.SplitHostPort(host); err == nil {
				host = hostname
			}
			return e.state.Services.ExposeDomain.Label(host)
		}
	}
	return "", false
}
func relayPage(conn net.Conn, status int, text string) {
	answer := &http.Response{StatusCode: status, ProtoMajor: 1, ProtoMinor: 1, Header: http.Header{"Content-Type": {"text/plain; charset=utf-8"}}, Body: io.NopCloser(strings.NewReader(text)), ContentLength: int64(len(text)), Close: true}
	// A disconnected visitor has nobody to receive another failure.
	_ = answer.Write(conn)
}
func (e *Edge) relay(ctx context.Context, conn net.Conn, input *bufio.Reader, head []byte, label string) (next *bufio.Reader, reusable bool) {
	notFound := func() {
		relayPage(conn, 404, "This expose does not exist (anymore); its URL is gone or has expired.\n")
	}
	unavailable := func() { relayPage(conn, 503, "This expose cannot be served right now; retry shortly.\n") }
	id, err := webapi.ParseExposeID(label)
	if err != nil {
		notFound()
		return
	}
	record, err := e.state.Services.Control.Expose(ctx, id)
	if err != nil {
		unavailable()
		return
	}
	if record == nil {
		notFound()
		return
	}
	shard, err := e.state.Shards.Of(ctx, record.User)
	if err != nil {
		unavailable()
		return
	}
	admitted, err := shard.OpenExposeConnection(ctx, id)
	if err != nil {
		var unreachable *expose.UnreachableError
		switch {
		case errors.Is(err, expose.ErrRelayNotFound):
			notFound()
		case errors.Is(err, expose.ErrLimit):
			relayPage(conn, 503, "This expose is at its concurrent-connection limit; retry shortly.\n")
		case errors.Is(err, expose.ErrRelayDeviceOffline):
			relayPage(conn, 502, "The exposed service is unreachable (device_offline).\n")
		case errors.Is(err, expose.ErrRemoved):
			relayPage(conn, 502, "This expose was removed while serving.\n")
		case errors.As(err, &unreachable):
			relayPage(conn, 502, fmt.Sprintf("The exposed service is unreachable (%s).\n", unreachable.Code))
		default:
			unavailable()
		}
		return
	}
	defer admitted.Lease.Release()
	lifetime, cancel := context.WithCancel(ctx)
	defer cancel()
	stream := &relayPipe{ctx: lifetime, cancel: cancel, reader: admitted.FromService, writer: admitted.ToService}
	// The operation reports IO failures; cleanup has no further recipient.
	defer func() { _ = stream.Close() }()
	watched := make(chan struct{})
	go func() {
		defer close(watched)
		select {
		case <-admitted.Lease.Context().Done():
			cancel()
			_ = conn.Close()
		case <-lifetime.Done():
		}
	}()
	defer func() {
		cancel()
		<-watched
	}()
	return forwardRelay(lifetime, conn, input, head, stream, string(record.Address), e.state.Services.ExposeTuning.Idle)
}

type relayPipe struct {
	ctx    context.Context
	cancel context.CancelFunc
	reader *remotehost.PipeReader
	writer *remotehost.PipeWriter
}

func (p *relayPipe) Read(b []byte) (int, error) { return p.reader.Read(p.ctx, b) }
func (p *relayPipe) Write(b []byte) (int, error) {
	if err := p.writer.Write(p.ctx, b); err != nil {
		return 0, err
	}
	return len(b), nil
}
func (p *relayPipe) Close() error {
	p.cancel()
	p.writer.Fail("relay ended")
	return p.reader.Close(context.WithoutCancel(p.ctx))
}

// relayHeaders preserves the spelling and separate values from the incoming
// head while removing connection-local fields and those nominated by Connection.
func relayHeaders(head []byte, upgrade bool) http.Header {
	original := originalHeaders(head)
	removed := map[string]bool{"connection": true, "keep-alive": true, "proxy-connection": true, "proxy-authenticate": true, "proxy-authorization": true, "te": true, "trailer": true, "transfer-encoding": true, "upgrade": true}
	for _, header := range original {
		if strings.EqualFold(header.name, "connection") {
			for _, token := range strings.Split(header.value, ",") {
				removed[strings.ToLower(strings.TrimSpace(token))] = true
			}
		}
	}
	if upgrade {
		delete(removed, "connection")
		delete(removed, "upgrade")
	}
	result := make(http.Header)
	for _, header := range original {
		if !removed[strings.ToLower(header.name)] {
			result[header.name] = append(result[header.name], header.value)
		}
	}
	return result
}
func deleteHeader(headers http.Header, name string) {
	for key := range headers {
		if strings.EqualFold(key, name) {
			delete(headers, key)
		}
	}
}
func wantsUpgrade(header http.Header) bool {
	for _, value := range header.Values("Connection") {
		for _, token := range strings.Split(value, ",") {
			if strings.EqualFold(strings.TrimSpace(token), "upgrade") && header.Get("Upgrade") != "" {
				return true
			}
		}
	}
	return false
}
func forwardRelay(ctx context.Context, visitor net.Conn, input *bufio.Reader, head []byte, service io.ReadWriteCloser, address string, idle time.Duration) (next *bufio.Reader, reusable bool) {
	next = bufio.NewReader(io.MultiReader(bytes.NewReader(head), input))
	request, err := http.ReadRequest(next)
	if err != nil {
		return
	}
	visitorClose := request.Close
	upgrade := wantsUpgrade(request.Header)
	https := overHTTPS(request)
	visitorHost := request.Host
	request.URL.Scheme = ""
	request.URL.Host = ""
	request.RequestURI = ""
	request.ProtoMajor = 1
	request.ProtoMinor = 1
	request.Header = relayHeaders(head, upgrade)
	deleteHeader(request.Header, "host")
	request.Header["Host"] = []string{address}
	deleteHeader(request.Header, "x-forwarded-host")
	deleteHeader(request.Header, "x-forwarded-proto")
	request.Host = address
	ip, _, err := net.SplitHostPort(visitor.RemoteAddr().String())
	if err != nil {
		ip = visitor.RemoteAddr().String()
	}
	request.Header["X-Forwarded-For"] = append(request.Header["X-Forwarded-For"], ip)
	request.Header["X-Forwarded-Host"] = []string{visitorHost}
	scheme := "http"
	if https {
		scheme = "https"
	}
	request.Header["X-Forwarded-Proto"] = []string{scheme}
	request.Close = !upgrade
	if !upgrade {
		request.Header["Connection"] = []string{"close"}
	}
	moved, ok := visitor.(*activity)
	if !ok {
		moved = newActivity(visitor)
	}
	stop := moved.watch(ctx, idle)
	defer stop()
	// Every relay owns and joins its request writer, including early answers.
	writing := make(chan struct{})
	var writeErr error
	go func() {
		defer close(writing)
		writeErr = writeRelayRequest(service, request)
	}()
	defer func() {
		_ = service.Close()
		if !reusable {
			_ = visitor.SetReadDeadline(time.Now())
		}
		<-writing
	}()
	from := bufio.NewReader(service)
	for {
		answerHead, err := readHead(from)
		if err != nil {
			relayPage(visitor, 502, "The exposed service is unreachable (no_response).\n")
			return
		}
		answerReader := bufio.NewReader(io.MultiReader(bytes.NewReader(answerHead), from))
		answer, err := http.ReadResponse(answerReader, request)
		if err != nil {
			relayPage(visitor, 502, "The exposed service is unreachable (no_response).\n")
			return
		}
		// The operation reports IO failures; cleanup has no further recipient.
		defer func() {
			// A visitor may have left midway through the response. End the
			// service pipe before Body.Close can try to drain its remaining bytes.
			_ = service.Close()
			_ = answer.Body.Close()
		}()
		if answer.StatusCode < 200 && answer.StatusCode != 101 {
			answer.Header = relayHeaders(answerHead, false)
			if err := writeRelayResponse(moved, answer); err != nil {
				return
			}
			from = answerReader
			continue
		}
		switched := upgrade && answer.StatusCode == 101
		answer.Header = relayHeaders(answerHead, switched)
		if switched {
			answer.Body = http.NoBody
			answer.ContentLength = 0
			answer.TransferEncoding = nil
			if err := writeRelayResponse(moved, answer); err != nil {
				return
			}
			copyDone := make(chan struct{})
			go func() {
				defer close(copyDone)
				_, _ = io.Copy(service, next)
				_ = service.Close()
			}()
			_, _ = io.Copy(moved, answerReader)
			_ = visitor.Close()
			_ = service.Close()
			<-copyDone
			return
		}
		if err := writeRelayResponse(moved, answer); err != nil {
			return
		}
		select {
		case <-writing:
			reusable = writeErr == nil && !visitorClose
		default:
			// An early answer leaves an unread upload; its connection cannot be reused.
		}
		return
	}
}

// writeRelayRequest uses net/http's parsed body framing and the original head's
// header spelling. Request.Write would add a User-Agent and duplicate a
// noncanonical Content-Length name instead of preserving the received field.
func writeRelayRequest(to io.Writer, request *http.Request) error {
	if _, err := fmt.Fprintf(to, "%s %s HTTP/1.1\r\n", request.Method, request.URL.RequestURI()); err != nil {
		return err
	}
	if len(request.TransferEncoding) != 0 {
		request.Header["Transfer-Encoding"] = []string{"chunked"}
	}
	if err := request.Header.Write(to); err != nil {
		return err
	}
	if _, err := io.WriteString(to, "\r\n"); err != nil {
		return err
	}
	if len(request.TransferEncoding) == 0 {
		_, err := io.Copy(to, request.Body)
		return err
	}
	chunks := httputil.NewChunkedWriter(to)
	if _, err := io.Copy(chunks, request.Body); err != nil {
		return err
	}
	if err := chunks.Close(); err != nil {
		return err
	}
	_, err := io.WriteString(to, "\r\n")
	return err
}

// writeRelayResponse preserves the service's spelling too, and frames an
// unknown-length body so the visitor connection may serve another request.
func writeRelayResponse(to io.Writer, answer *http.Response) error {
	noBody := answer.Request.Method == "HEAD" || answer.StatusCode < 200 || answer.StatusCode == 204 || answer.StatusCode == 304
	chunked := !noBody && answer.ContentLength < 0
	if chunked {
		answer.Header["Transfer-Encoding"] = []string{"chunked"}
	}
	if !noBody && answer.ContentLength >= 0 {
		hasLength := false
		for key := range answer.Header {
			if strings.EqualFold(key, "Content-Length") {
				hasLength = true
			}
		}
		if !hasLength {
			answer.Header["Content-Length"] = []string{strconv.FormatInt(answer.ContentLength, 10)}
		}
	}
	if _, err := fmt.Fprintf(to, "HTTP/1.1 %s\r\n", answer.Status); err != nil {
		return err
	}
	if err := answer.Header.Write(to); err != nil {
		return err
	}
	if _, err := io.WriteString(to, "\r\n"); err != nil {
		return err
	}
	if noBody {
		return nil
	}
	if !chunked {
		_, err := io.Copy(to, answer.Body)
		return err
	}
	chunks := httputil.NewChunkedWriter(to)
	if _, err := io.Copy(chunks, answer.Body); err != nil {
		return err
	}
	if err := chunks.Close(); err != nil {
		return err
	}
	_, err := io.WriteString(to, "\r\n")
	return err
}
