package commandservice

import (
	"context"
	"io"
	"net"
	"net/http"
	"os"
	"time"
)

// This file gives the tests of package commandservice_test the few internals
// they reach: a raw request of the wire, and a service in states its own client
// cannot bring about. The tests state the wire's limits in their own values, so
// that a limit that is wrong fails them.

// Send sends any request to a path of the wire, as a caller that does not keep
// the SDK's rules would.
func (c *Client) Send(ctx context.Context, method, path string, body io.ReadCloser, length int64) (*http.Response, error) {
	return c.send(ctx, method, path, body, length)
}

// Stream returns the stream that carries the records of the numbers stream.
func (s *NumbersStream) Stream() *Stream { return s.stream }

// DrainingService returns the HTTP handler of a service that was asked to shut
// down.
func DrainingService() http.Handler {
	s := &service{}
	s.draining.Store(true)
	return s
}

// StoppedService returns the HTTP handler of a service that stopped taking
// requests.
func StoppedService() http.Handler { return &service{stopped: true} }

// ServiceConn is the connection a service serves, which keeps what it saw of
// the peer.
type ServiceConn = serviceConn

// NewServiceConn returns the connection a service would serve for conn.
func NewServiceConn(conn net.Conn) *ServiceConn { return newServiceConn(conn) }

// PeerEnded reports whether the peer told the service that it closed.
func (c *ServiceConn) PeerEnded() bool { return c.peerEnded() }

// Failure returns the error that broke the connection, if the peer did.
func (c *ServiceConn) Failure() error { return c.failure() }

// ServiceHandler returns the HTTP handler of a service of handler that is not
// served over a connection, and the function that tells whether the service
// faulted.
func ServiceHandler(handler Handler) (http.Handler, func() error, error) {
	s, err := newService(context.Background(), handler)
	if err != nil {
		return nil, nil, err
	}
	return s, s.fault, nil
}

// ErrNoDescriptor is the error of a process that has no descriptor left, on
// this platform.
var ErrNoDescriptor = errNoDescriptor

// BirthTime returns when the file at path was created, when this system tells.
func BirthTime(path string) (time.Time, bool) {
	info, err := os.Stat(path)
	if err != nil {
		return time.Time{}, false
	}
	return birthTime(path, info)
}
