package commandservice

import (
	"context"
	"io"
	"net"
	"os"
	"os/signal"
	"sync"
	"syscall"

	"github.com/wspl/demi/go/commandservice/internal/pipeconn"
)

// ServeStdio serves the connection over the process's standard input and
// output, which its parent owns: the runner starts the executable and speaks
// HTTP/2 to it over its pipes. Nothing else may write to standard output,
// which carries only protocol bytes; diagnostics go to standard error.
//
// ServeStdio returns as [Serve] does. The executable owns the process
// lifetime: it should exit after ServeStdio returns, which also releases a read
// of standard input that is still waiting on the parent.
func ServeStdio(ctx context.Context, handler Handler) error {
	// A write to a pipe its reader closed ends a process with SIGPIPE, but a
	// service must see that write fail: a peer that closes after an answered
	// shutdown is not a failure. A signal that is caught, unlike one that is
	// ignored, is reset to its default in the processes a handler starts.
	sigpipe := make(chan os.Signal, 1)
	signal.Notify(sigpipe, syscall.SIGPIPE)
	defer signal.Stop(sigpipe)
	return Serve(ctx, pipeconn.New(os.Stdin, os.Stdout), handler)
}

// A serviceConn is the connection a service serves. It says when the
// connection closed, and keeps what it saw of the peer: whether the peer told
// the service that it closed, and the error that broke the connection, if the
// peer did.
type serviceConn struct {
	net.Conn
	closed    chan struct{}
	closeOnce sync.Once

	mu     sync.Mutex
	ended  bool
	broken error
}

func newServiceConn(conn net.Conn) *serviceConn {
	return &serviceConn{Conn: conn, closed: make(chan struct{})}
}

func (c *serviceConn) Read(p []byte) (int, error) {
	n, err := c.Conn.Read(p)
	if err != nil && !c.isClosed() {
		// A read that fails after the service closed the connection fails
		// because of that, and says nothing of the peer.
		c.mu.Lock()
		c.ended = true
		c.mu.Unlock()
		if err != io.EOF {
			c.note(err)
		}
	}
	return n, err
}

func (c *serviceConn) Write(p []byte) (int, error) {
	n, err := c.Conn.Write(p)
	if err != nil {
		c.note(err)
	}
	return n, err
}

// note keeps the first error of the peer's side; an error after the service
// closed the connection itself is not the peer's.
func (c *serviceConn) note(err error) {
	if c.isClosed() {
		return
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.broken == nil {
		c.broken = err
	}
}

// isClosed reports whether the service closed the connection.
func (c *serviceConn) isClosed() bool {
	select {
	case <-c.closed:
		return true
	default:
		return false
	}
}

func (c *serviceConn) Close() error {
	c.closeOnce.Do(func() { close(c.closed) })
	return c.Conn.Close()
}

// peerEnded reports whether the peer told the service that it closed: a read
// of the connection ended, with the end of input or with an error, while the
// service had not closed it.
func (c *serviceConn) peerEnded() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.ended
}

// failure returns the error that broke the connection, or nil when it ended
// cleanly.
func (c *serviceConn) failure() error {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.broken
}

// A connListener hands its one connection to an [net/http.Server] and then
// waits to be closed.
type connListener struct {
	conn net.Conn

	mu        sync.Mutex
	accepted  bool
	closed    chan struct{}
	closeOnce sync.Once
}

func newConnListener(conn net.Conn) *connListener {
	return &connListener{conn: conn, closed: make(chan struct{})}
}

func (l *connListener) Accept() (net.Conn, error) {
	l.mu.Lock()
	first := !l.accepted
	l.accepted = true
	l.mu.Unlock()
	if first {
		return l.conn, nil
	}
	<-l.closed
	return nil, net.ErrClosed
}

func (l *connListener) Close() error {
	l.closeOnce.Do(func() {
		close(l.closed)
		l.mu.Lock()
		unclaimed := !l.accepted
		l.accepted = true
		l.mu.Unlock()
		if unclaimed {
			// The server never took the connection, so nothing else closes it.
			_ = l.conn.Close()
		}
	})
	return nil
}

func (l *connListener) Addr() net.Addr { return l.conn.LocalAddr() }
