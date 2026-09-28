package commandservice

import (
	"context"
	"errors"
	"net"
	"os"
	"sync"
	"time"
)

// pipeConn preserves the two independently owned standard IO pipe halves.
type pipeConn struct {
	in, out *os.File
	once    sync.Once
}

func (p *pipeConn) Read(b []byte) (int, error) { return p.in.Read(b) }

func (p *pipeConn) Write(b []byte) (int, error) { return p.out.Write(b) }

// Close releases both pipe halves.
func (p *pipeConn) Close() error {
	var err error
	p.once.Do(func() {
		readErr := p.in.Close()
		writeErr := p.out.Close()
		err = errors.Join(readErr, writeErr)
	})
	return err
}

func (p *pipeConn) LocalAddr() net.Addr { return pipeAddress("stdio") }

func (p *pipeConn) RemoteAddr() net.Addr { return pipeAddress("runner") }

func (p *pipeConn) SetDeadline(t time.Time) error {
	if err := p.SetReadDeadline(t); err != nil {
		return err
	}
	return p.SetWriteDeadline(t)
}

func (p *pipeConn) SetReadDeadline(t time.Time) error { return p.in.SetReadDeadline(t) }

func (p *pipeConn) SetWriteDeadline(t time.Time) error { return p.out.SetWriteDeadline(t) }

type pipeAddress string

func (a pipeAddress) Network() string { return "pipe" }

func (a pipeAddress) String() string { return string(a) }

// ServeStdio serves a handler over standard input and output, writing only protocol bytes
// to stdout.
func ServeStdio(ctx context.Context, h Handler) error {
	return Serve(ctx, &pipeConn{in: os.Stdin, out: os.Stdout}, h)
}
