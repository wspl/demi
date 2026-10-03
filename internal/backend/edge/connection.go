package edge

import (
	"context"
	"io"
	"net"
	"time"
)

// incoming owns read-ahead on a visitor connection. A transport EOF cancels
// work waiting on a Host even when that work is not reading the request body.
// Close releases the reader and joins the pump before its owner returns.
type incoming struct {
	net.Conn
	reader *io.PipeReader
	cancel context.CancelFunc
	done   chan struct{}
}

func readIncoming(ctx context.Context, conn net.Conn) (context.Context, *incoming) {
	ctx, cancel := context.WithCancel(ctx)
	reader, writer := io.Pipe()
	c := &incoming{Conn: conn, reader: reader, cancel: cancel, done: make(chan struct{})}
	go func() {
		defer close(c.done)
		defer cancel()
		_, err := io.Copy(writer, conn)
		_ = writer.CloseWithError(err)
	}()
	return ctx, c
}
func (c *incoming) Read(p []byte) (int, error) { return c.reader.Read(p) }
func (c *incoming) Close() error {
	c.cancel()
	_ = c.reader.Close()
	err := c.Conn.Close()
	<-c.done
	return err
}

// CloseWrite forwards TCP's half-close through the read-ahead owner.
func (c *incoming) CloseWrite() error {
	if writer, ok := c.Conn.(interface{ CloseWrite() error }); ok {
		return writer.CloseWrite()
	}
	return nil
}

// closeWriteAndWait lets an uploading visitor read the already-flushed refusal
// before a full close can reset TCP. net/http's closeWriteAndWait uses a 500 ms
// rstAvoidanceDelay; use that same grace, discarding input until EOF or expiry.
func closeWriteAndWait(ctx context.Context, conn net.Conn) {
	if ctx.Err() != nil {
		return
	}
	if writer, ok := conn.(interface{ CloseWrite() error }); ok {
		if err := writer.CloseWrite(); err != nil {
			return
		}
	}
	if err := conn.SetReadDeadline(time.Now().Add(500 * time.Millisecond)); err != nil {
		return
	}
	// EOF, deadline and transport errors all end this best-effort grace. The
	// connection owner closes the transport and joins its read-ahead worker next.
	_, _ = io.Copy(io.Discard, conn)
}
