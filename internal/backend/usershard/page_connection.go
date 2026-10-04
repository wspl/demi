package usershard

import (
	"context"
	"io"
	"time"

	"github.com/coder/websocket"
)

// PageConnection owns a page's WebSocket and the transport its close deadline ends.
type PageConnection struct {
	// Conn provides WebSocket framing; Close bounds its transport lifetime.
	*websocket.Conn
	transport io.Closer
}

// NewPageConnection takes ownership of socket and its underlying transport.
// The caller releases the connection with Close or CloseNow.
func NewPageConnection(socket *websocket.Conn, transport io.Closer) *PageConnection {
	return &PageConnection{Conn: socket, transport: transport}
}

// Close sends the close frame and joins the handshake, cutting the underlying
// transport when wait expires. A page that stopped reading cannot extend the wait.
func (p *PageConnection) Close(ctx context.Context, code websocket.StatusCode, reason string, wait time.Duration) {
	closed := make(chan struct{})
	closeCtx, cancel := context.WithTimeout(ctx, wait)
	defer cancel()
	stop := context.AfterFunc(closeCtx, func() {
		defer close(closed)
		// CloseNow only joins an in-progress WebSocket close. End its IO
		// directly; errors are immaterial when abandoning the transport.
		_ = p.transport.Close()
	})
	// A broken peer cannot use the close frame; either way its transport is released.
	_ = p.Conn.Close(code, reason)
	if !stop() {
		<-closed
	}
	// The handshake has ended; release the framing reader as well.
	_ = p.CloseNow()
}
