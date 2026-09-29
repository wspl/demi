// Package pipeconn adapts a pair of pipes to a [net.Conn]: the standard input
// and output of a service, or the other ends of a child process's.
package pipeconn

import (
	"errors"
	"net"
	"os"
	"time"
)

// New returns a connection that reads from read and writes to write. Closing
// it closes both. A pipe that a parent handed over may not support deadlines:
// setting one on the connection then fails, and clearing one succeeds.
func New(read, write *os.File) net.Conn {
	return conn{read: read, write: write}
}

type conn struct {
	read  *os.File
	write *os.File
}

func (c conn) Read(p []byte) (int, error)  { return c.read.Read(p) }
func (c conn) Write(p []byte) (int, error) { return c.write.Write(p) }

func (c conn) Close() error {
	return errors.Join(c.write.Close(), c.read.Close())
}

func (conn) LocalAddr() net.Addr  { return addr{} }
func (conn) RemoteAddr() net.Addr { return addr{} }

func (c conn) SetDeadline(t time.Time) error {
	return errors.Join(c.SetReadDeadline(t), c.SetWriteDeadline(t))
}

func (c conn) SetReadDeadline(t time.Time) error {
	return clearing(c.read.SetReadDeadline(t), t)
}

func (c conn) SetWriteDeadline(t time.Time) error {
	return clearing(c.write.SetWriteDeadline(t), t)
}

// clearing lets a pipe that has no deadlines accept the request to clear one.
func clearing(err error, deadline time.Time) error {
	if deadline.IsZero() && errors.Is(err, os.ErrNoDeadline) {
		return nil
	}
	return err
}

type addr struct{}

func (addr) Network() string { return "pipe" }
func (addr) String() string  { return "pipe" }
