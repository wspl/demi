package httpserver

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
func (w *response) Header() http.Header { return w.header }

// WriteHeader starts the encoder after deciding whether the connection can be reused.
func (w *response) WriteHeader(status int) {
	if w.status != 0 || w.hijacked {
		return
	}
	w.status = status
	// Decide before publishing headers: an unread body cannot be reused without
	// draining, which could wait forever for a refused upload. The connection
	// loop honors this same decision even if the handler later finishes reading.
	if status != http.StatusSwitchingProtocols {
		w.closeAfterReply = w.request.Close || !w.requestBody.complete ||
			websocketToken(w.header, "Connection", "close")
		if w.closeAfterReply {
			w.header.Set("Connection", "close")
		}
	}
	reader, writer := io.Pipe()
	w.body = writer
	w.done = make(chan error, 1)
	answer := &http.Response{
		StatusCode:    status,
		ProtoMajor:    1,
		ProtoMinor:    1,
		Header:        w.header.Clone(),
		Body:          reader,
		ContentLength: -1,
		Request:       w.request,
		Close:         w.closeAfterReply,
	}
	if length := w.header.Get("Content-Length"); length != "" {
		if size, err := strconv.ParseInt(length, 10, 64); err == nil {
			answer.ContentLength = size
		}
	}
	if answer.ContentLength < 0 {
		answer.TransferEncoding = []string{"chunked"}
	}
	if w.request.Method == http.MethodHead || status < 200 || status == 204 || status == 304 {
		answer.TransferEncoding = nil
		if answer.ContentLength < 0 {
			answer.ContentLength = 0
		}
		answer.Body = http.NoBody
	}
	go func() {
		err := answer.Write(w.conn)
		// Wake a producer if the visitor leaves or this status has no body.
		_ = reader.CloseWithError(err)
		w.done <- err
	}()
}

// Write streams response bytes, respecting statuses and methods with no body.
func (w *response) Write(p []byte) (int, error) {
	if w.hijacked {
		return 0, http.ErrHijacked
	}
	if w.status == 0 {
		w.WriteHeader(200)
	}
	if w.request.Method == http.MethodHead {
		return len(p), nil
	}
	return w.body.Write(p)
}

// Flush commits headers if no response has started.
func (w *response) Flush() {
	if w.status == 0 {
		w.WriteHeader(200)
	}
}

func (w *response) finish() error {
	if w.hijacked {
		return nil
	}
	if w.status == 0 {
		w.WriteHeader(200)
	}
	// Closing a pipe writer is infallible here; the encoder reports wire errors.
	_ = w.body.Close()
	return <-w.done
}

// Hijack finishes an upgrade response before handing the connection to its new owner.
func (w *response) Hijack() (net.Conn, *bufio.ReadWriter, error) {
	if w.status != 0 && w.status != 101 || w.hijacked {
		return nil, nil, errors.New("response already started")
	}
	if w.status == 101 {
		if err := w.finish(); err != nil {
			return nil, nil, err
		}
	}
	w.hijacked = true
	return w.conn, bufio.NewReadWriter(w.input, bufio.NewWriter(w.conn)), nil
}

// SetReadDeadline forwards the HTTP controller deadline to the connection.
func (w *response) SetReadDeadline(t time.Time) error { return w.conn.SetReadDeadline(t) }

// SetWriteDeadline forwards the HTTP controller deadline to the connection.
func (w *response) SetWriteDeadline(t time.Time) error { return w.conn.SetWriteDeadline(t) }
