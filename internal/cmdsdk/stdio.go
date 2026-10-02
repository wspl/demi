package cmdsdk

import (
	"context"
	"errors"
	"net"
	"os"
	"time"

	"github.com/wspl/demi/internal/commandwire"
)

// ServeStdio takes ownership of the executable's protocol stdin and stdout.
// The executable must exit after this returns.
func ServeStdio(ctx context.Context, h Handler[commandwire.Invocation]) error {
	return Serve(ctx, &PipeConn{Reader: os.Stdin, Writer: os.Stdout}, h)
}

// PipeConn adapts owned input/output files to a duplex HTTP/2 transport.
// Close releases both files; callers must not use them afterward.
type PipeConn struct{ Reader, Writer *os.File }

func (c *PipeConn) Read(b []byte) (int, error)  { return c.Reader.Read(b) }
func (c *PipeConn) Write(b []byte) (int, error) { return c.Writer.Write(b) }

// Close releases both pipe ends.
func (c *PipeConn) Close() error { return errors.Join(c.Reader.Close(), c.Writer.Close()) }

// LocalAddr identifies the local pipe end.
func (c *PipeConn) LocalAddr() net.Addr { return &net.UnixAddr{Name: "local", Net: "pipe"} }

// RemoteAddr identifies the peer pipe end.
func (c *PipeConn) RemoteAddr() net.Addr { return &net.UnixAddr{Name: "peer", Net: "pipe"} }

// SetDeadline sets both pipe deadlines.
func (c *PipeConn) SetDeadline(t time.Time) error {
	return errors.Join(c.SetReadDeadline(t), c.SetWriteDeadline(t))
}

// SetReadDeadline sets the input pipe deadline.
func (c *PipeConn) SetReadDeadline(t time.Time) error { return c.Reader.SetReadDeadline(t) }

// SetWriteDeadline sets the output pipe deadline.
func (c *PipeConn) SetWriteDeadline(t time.Time) error { return c.Writer.SetWriteDeadline(t) }
