package usershard

import (
	"context"
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
	socket.SetReadLimit(webapi.MaxPageMessageBytes)
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
			kind, data, err := socket.Read(ctx)
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
