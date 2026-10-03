package usershard

import (
	"context"
	"io"
	"sync"
	"time"

	"github.com/coder/websocket"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapi"
)

type pageMessage struct {
	kind websocket.MessageType
	data []byte
	err  error
}
type pageSocket struct {
	socket    *websocket.Conn
	tuning    PageTuning
	heartbeat *time.Timer
}

func newPageSocket(socket *websocket.Conn, tuning PageTuning) *pageSocket {
	return &pageSocket{socket: socket, tuning: tuning, heartbeat: time.NewTimer(tuning.Heartbeat)}
}
func (p *pageSocket) send(ctx context.Context, value any) error {
	data, err := contract.EncodeJSON(value)
	if err != nil {
		return err
	}
	if err = p.socket.Write(ctx, websocket.MessageText, data); err != nil {
		return err
	}
	p.heartbeat.Reset(p.tuning.Heartbeat)
	return nil
}
func (p *pageSocket) close(ctx context.Context, code websocket.StatusCode, reason string) {
	p.heartbeat.Stop()
	// Close has no context; the owner interrupts its handshake after CloseWait.
	closed := make(chan struct{})
	closeCtx, cancel := context.WithTimeout(ctx, p.tuning.CloseWait)
	defer cancel()
	stop := context.AfterFunc(closeCtx, func() {
		_ = p.socket.CloseNow()
		close(closed)
	})
	_ = p.socket.Close(code, reason)
	if !stop() {
		<-closed
	}
	_ = p.socket.CloseNow()
}
func readPage(ctx context.Context, socket *websocket.Conn, workers *sync.WaitGroup) <-chan pageMessage {
	incoming := make(chan pageMessage)
	workers.Add(1)
	go func() {
		defer workers.Done()
		defer close(incoming)
		for {
			kind, data, err := ReadPageMessage(ctx, socket)
			select {
			case incoming <- pageMessage{kind: kind, data: data, err: err}:
			case <-ctx.Done():
				return
			}
			if err != nil {
				return
			}
		}
	}()
	return incoming
}

// ReadPageMessage reads one bounded conversation, sync or user-stream message.
// An oversized message ends the transport without sending a WebSocket close code.
func ReadPageMessage(ctx context.Context, socket *websocket.Conn) (websocket.MessageType, []byte, error) {
	// coder/websocket's own limit sends 1009 and cannot meet web-api.md's
	// no-close-code rule. Keep its framing reader, but bound the message here.
	socket.SetReadLimit(-1)
	kind, reader, err := socket.Reader(ctx)
	if err != nil {
		return kind, nil, err
	}
	data, err := io.ReadAll(io.LimitReader(reader, webapi.MaxPageMessageBytes+1))
	if len(data) > webapi.MaxPageMessageBytes {
		// Overflow already failed the message; a close error needs no recovery.
		_ = socket.CloseNow()
		return kind, nil, websocket.ErrMessageTooBig
	}
	return kind, data, err
}
