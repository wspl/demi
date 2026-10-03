package edge

import (
	"context"
	"io"
	"net"
	"sync/atomic"
	"time"
)

// incoming owns read-ahead on a visitor connection. A transport EOF cancels
// work waiting on a Host even when that work is not reading the request body,
// except after an upgrade allows the reverse direction to finish independently.
// Close releases the reader and joins the pump before its owner returns.
type incoming struct {
	net.Conn
	reader           *io.PipeReader
	cancel           context.CancelFunc
	done             chan struct{}
	halfCloseAllowed atomic.Bool
}

func readIncoming(ctx context.Context, conn net.Conn) (context.Context, *incoming) {
	ctx, cancel := context.WithCancel(ctx)
	reader, writer := io.Pipe()
	i := &incoming{Conn: conn, reader: reader, cancel: cancel, done: make(chan struct{})}
	go func() {
		defer close(i.done)
		defer func() {
			if !i.halfCloseAllowed.Load() {
				cancel()
			}
		}()
		_, err := io.Copy(writer, conn)
		_ = writer.CloseWithError(err)
	}()
	return ctx, i
}

// Read consumes bytes from the owned read-ahead worker.
func (i *incoming) Read(p []byte) (int, error) { return i.reader.Read(p) }

// Close cancels the connection and joins its read-ahead worker.
func (i *incoming) Close() error {
	i.cancel()
	_ = i.reader.Close()
	err := i.Conn.Close()
	<-i.done
	return err
}

// CloseWrite forwards TCP's half-close through the read-ahead owner.
func (i *incoming) CloseWrite() error {
	if writer, ok := i.Conn.(interface{ CloseWrite() error }); ok {
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

// allowHalfClose lets an upgraded relay finish writing after visitor read EOF.
// Explicit Close and parent shutdown still cancel the connection's context.
func (i *incoming) allowHalfClose() { i.halfCloseAllowed.Store(true) }
