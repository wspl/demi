package httpserver

import (
	"io"
	"net"
	"net/http"
	"strings"
)

// requestBody records whether the next head can safely be read on this
// connection and sends 100 Continue only when a handler starts taking bytes.
type requestBody struct {
	io.ReadCloser
	conn     net.Conn
	expect   bool
	complete bool
}

func trackBody(request *http.Request, conn net.Conn) *requestBody {
	body := &requestBody{
		ReadCloser: request.Body,
		conn:       conn,
		expect:     strings.EqualFold(request.Header.Get("Expect"), "100-continue"),
		complete:   request.Body == http.NoBody,
	}
	request.Body = body
	return body
}

// Read sends a requested 100 Continue before the first body read and tracks completion.
func (b *requestBody) Read(p []byte) (int, error) {
	if b.expect {
		b.expect = false
		if _, err := io.WriteString(b.conn, "HTTP/1.1 100 Continue\r\n\r\n"); err != nil {
			return 0, err
		}
	}
	n, err := b.ReadCloser.Read(p)
	if err == io.EOF {
		b.complete = true
	}
	return n, err
}
