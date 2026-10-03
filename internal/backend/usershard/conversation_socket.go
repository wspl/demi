package usershard

import (
	"context"
	"errors"
	"log/slog"
	"sync"

	"github.com/coder/websocket"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/framewire"
)

func (s *Shard) serveConversation(ctx context.Context, record database.ConversationRecord, socket *websocket.Conn) error {
	// Closing an already broken socket needs no recovery.
	defer func() { _ = socket.CloseNow() }()
	_, err := shardCall(ctx, s, func(ctx context.Context) (struct{}, error) {
		return struct{}{}, s.relayConversation(ctx, record, socket)
	})
	return err
}
func (s *Shard) relayConversation(ctx context.Context, record database.ConversationRecord, socket *websocket.Conn) error {
	ctx, cancel := context.WithCancel(ctx)
	stopped := make(chan struct{})
	stop := context.AfterFunc(s.ctx, func() {
		cancel()
		close(stopped)
	})
	defer func() {
		if !stop() {
			<-stopped
		}
	}()
	defer cancel()
	target, err := hostaccess.ResolveTarget(ctx, s, record)
	if err != nil {
		slog.ErrorContext(ctx, "the conversation's target does not resolve", "conversation", record.ID, "error", err)
		return err
	}
	files := &conversationFiles{shard: s, conversation: record.ID}
	connection, frames := s.agent.Connect(hostaccess.RootOf(record.ID), database.ExecutionPath(target), files)
	defer connection.Detach()
	page := newPageSocket(socket, s.services.Pages)
	defer page.heartbeat.Stop()
	var workers sync.WaitGroup
	defer func() {
		cancel()
		_ = socket.CloseNow()
		workers.Wait()
	}()
	readCtx, readCancel := context.WithCancel(context.WithoutCancel(ctx))
	defer readCancel()
	incoming := readPage(readCtx, socket, &workers)
	outgoing := make(chan server.Outgoing)
	workers.Add(1)
	go func() {
		defer workers.Done()
		defer close(outgoing)
		for {
			frame, err := frames.Receive(ctx)
			if err != nil {
				return
			}
			select {
			case outgoing <- frame:
			case <-ctx.Done():
				return
			}
			if _, closed := frame.(*server.Closed); closed {
				return
			}
		}
	}()
	type handled struct {
		reply framewire.ServerFrame
		err   error
	}
	var handling <-chan handled
	defer func() {
		if handling != nil {
			<-handling
		}
	}()
	send := func(frame framewire.ServerFrame) error {
		frame, err := s.present(ctx, frame)
		if err != nil {
			return err
		}
		return page.send(ctx, frame)
	}
	for {
		reading := incoming
		if handling != nil {
			reading = nil
		}
		select {
		case <-ctx.Done():
			page.close(context.WithoutCancel(ctx), websocket.StatusGoingAway, "backend_closing")
			return ctx.Err()
		case message, ok := <-reading:
			if !ok || message.err != nil {
				return message.err
			}
			if message.kind != websocket.MessageText {
				page.close(context.WithoutCancel(ctx), websocket.StatusInvalidFramePayloadData, "not_json")
				return nil
			}
			result := make(chan handled, 1)
			handling = result
			go func() {
				reply, err := s.handleMessage(context.WithoutCancel(ctx), record.ID, connection, files, message.data)
				result <- handled{reply: reply, err: err}
			}()
		case result := <-handling:
			handling = nil
			if errors.Is(result.err, framewire.ErrNotJSON) {
				page.close(context.WithoutCancel(ctx), websocket.StatusInvalidFramePayloadData, "not_json")
				return nil
			}
			if result.err != nil {
				return result.err
			}
			if result.reply != nil {
				if err := send(result.reply); err != nil {
					return err
				}
			}
		case item, ok := <-outgoing:
			if !ok {
				return nil
			}
			switch item := item.(type) {
			case *server.Frame:
				if err := send(item.Frame); err != nil {
					return err
				}
			case *server.Lagged:
				page.close(context.WithoutCancel(ctx), websocket.StatusCode(4001), "lagged")
				return nil
			case *server.Closed:
				return nil
			}
		case <-page.heartbeat.C:
			if err := page.send(ctx, &framewire.HeartbeatFrame{}); err != nil {
				return err
			}
		}
	}
}
