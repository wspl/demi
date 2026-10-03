package edge

import (
	"context"
	"io"
	"net"
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
