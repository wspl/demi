package edge

import (
	"bufio"
	"errors"
	"io"
	"net"
	"net/http"
	"strconv"
	"time"
)

// response streams a single response through net/http's HTTP/1 encoder. The
// connection owner joins the encoder before reading the next request head.
type response struct {
	conn            net.Conn
	input           *bufio.Reader
	request         *http.Request
	requestBody     *requestBody
	closeAfterReply bool
	header          http.Header
	body            *io.PipeWriter
	done            chan error
	status          int
	hijacked        bool
}

// Header returns the headers to commit for this response.
func (r *response) Header() http.Header { return r.header }

// WriteHeader starts the encoder after deciding whether the connection can be reused.
func (r *response) WriteHeader(status int) {
	if r.status != 0 || r.hijacked {
		return
	}
	r.status = status
	// Decide before publishing headers: an unread body cannot be reused without
	// draining, which could wait forever for a refused upload. The connection
	// loop honors this same decision even if the handler later finishes reading.
	if status != http.StatusSwitchingProtocols {
		r.closeAfterReply = r.request.Close || !r.requestBody.complete ||
			websocketToken(r.header, "Connection", "close")
		if r.closeAfterReply {
			r.header.Set("Connection", "close")
		}
	}
	reader, writer := io.Pipe()
	r.body = writer
	r.done = make(chan error, 1)
	answer := &http.Response{
		StatusCode:    status,
		ProtoMajor:    1,
		ProtoMinor:    1,
		Header:        r.header.Clone(),
		Body:          reader,
		ContentLength: -1,
		Request:       r.request,
		Close:         r.closeAfterReply,
	}
	if length := r.header.Get("Content-Length"); length != "" {
		if size, err := strconv.ParseInt(length, 10, 64); err == nil {
			answer.ContentLength = size
		}
	}
	if answer.ContentLength < 0 {
		answer.TransferEncoding = []string{"chunked"}
	}
	if r.request.Method == http.MethodHead || status < 200 || status == 204 || status == 304 {
		answer.TransferEncoding = nil
		if answer.ContentLength < 0 {
			answer.ContentLength = 0
		}
		answer.Body = http.NoBody
	}
	go func() {
		err := answer.Write(r.conn)
		// Wake a producer if the visitor leaves or this status has no body.
		_ = reader.CloseWithError(err)
		r.done <- err
	}()
}

// Write streams response bytes, respecting statuses and methods with no body.
func (r *response) Write(p []byte) (int, error) {
	if r.hijacked {
		return 0, http.ErrHijacked
	}
	if r.status == 0 {
		r.WriteHeader(200)
	}
	if r.request.Method == http.MethodHead {
		return len(p), nil
	}
	return r.body.Write(p)
}

// Flush commits headers if no response has started.
func (r *response) Flush() {
	if r.status == 0 {
		r.WriteHeader(200)
	}
}

func (r *response) finish() error {
	if r.hijacked {
		return nil
	}
	if r.status == 0 {
		r.WriteHeader(200)
	}
	// Closing a pipe writer is infallible here; the encoder reports wire errors.
	_ = r.body.Close()
	return <-r.done
}

// Hijack finishes an upgrade response before handing the connection to its new owner.
func (r *response) Hijack() (net.Conn, *bufio.ReadWriter, error) {
	if r.status != 0 && r.status != 101 || r.hijacked {
		return nil, nil, errors.New("response already started")
	}
	if r.status == 101 {
		if err := r.finish(); err != nil {
			return nil, nil, err
		}
	}
	r.hijacked = true
	return r.conn, bufio.NewReadWriter(r.input, bufio.NewWriter(r.conn)), nil
}

// SetReadDeadline forwards the HTTP controller deadline to the connection.
func (r *response) SetReadDeadline(t time.Time) error { return r.conn.SetReadDeadline(t) }

// SetWriteDeadline forwards the HTTP controller deadline to the connection.
func (r *response) SetWriteDeadline(t time.Time) error { return r.conn.SetWriteDeadline(t) }
