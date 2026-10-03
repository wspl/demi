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
	conn     net.Conn
	input    *bufio.Reader
	request  *http.Request
	header   http.Header
	body     *io.PipeWriter
	done     chan error
	status   int
	hijacked bool
}

func (w *response) Header() http.Header { return w.header }
func (w *response) WriteHeader(status int) {
	if w.status != 0 || w.hijacked {
		return
	}
	w.status = status
	reader, writer := io.Pipe()
	w.body = writer
	w.done = make(chan error, 1)
	answer := &http.Response{StatusCode: status, ProtoMajor: 1, ProtoMinor: 1, Header: w.header.Clone(), Body: reader, ContentLength: -1, Request: w.request, Close: w.request.Close}
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
func (w *response) SetReadDeadline(t time.Time) error  { return w.conn.SetReadDeadline(t) }
func (w *response) SetWriteDeadline(t time.Time) error { return w.conn.SetWriteDeadline(t) }
