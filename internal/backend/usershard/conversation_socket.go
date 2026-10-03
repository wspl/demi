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
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/webapi"
)

func (s *Shard) serveConversationSocket(
	ctx context.Context,
	record database.ConversationRecord,
	socket *websocket.Conn,
) error {
	// Closing an already broken socket needs no recovery.
	defer func() {
		_ = socket.CloseNow()
	}()
	_, err := shardCall(ctx, s, func(ctx context.Context) (struct{}, error) {
		return struct{}{}, s.relayConversation(ctx, record, socket)
	})
	return err
}

func (s *Shard) relayConversation(
	ctx context.Context,
	record database.ConversationRecord,
	socket *websocket.Conn,
) error {
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
	outgoing := make(chan framewire.ServerFrame)
	workers.Add(1)
	go forwardConversationFrames(ctx, frames, outgoing, &workers)
	return s.exchangeConversation(ctx, conversationExchange{
		id:         record.ID,
		connection: connection,
		files:      files,
		page:       page,
		incoming:   incoming,
		outgoing:   outgoing,
		frames:     frames,
	})
}

// forwardConversationFrames relays agent frames until cancellation or the outbox ends.
func forwardConversationFrames(
	ctx context.Context,
	frames *server.Frames,
	outgoing chan<- framewire.ServerFrame,
	workers *sync.WaitGroup,
) {
	defer workers.Done()
	defer close(outgoing)
	for frames.Next(ctx) {
		select {
		case outgoing <- frames.Frame():
		case <-ctx.Done():
			return
		}
	}
}

type handled struct {
	reply framewire.ServerFrame
	err   error
}

type conversationExchange struct {
	id         webapi.ConversationID
	connection *server.Connection[*remotehost.Host]
	files      *conversationFiles
	page       *pageSocket
	incoming   <-chan pageMessage
	outgoing   <-chan framewire.ServerFrame
	frames     *server.Frames // Read Err only after outgoing closes.
}

// exchangeConversation sends agent frames while admitting one client message at a time.
func (s *Shard) exchangeConversation(ctx context.Context, exchange conversationExchange) error {
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
		return exchange.page.send(ctx, frame)
	}
	for {
		reading := exchange.incoming
		if handling != nil {
			reading = nil
		}
		select {
		case <-ctx.Done():
			exchange.page.close(context.WithoutCancel(ctx), websocket.StatusGoingAway, "backend_closing")
			return ctx.Err()
		case message, ok := <-reading:
			if !ok || message.err != nil {
				return message.err
			}
			if message.kind != websocket.MessageText {
				exchange.page.close(context.WithoutCancel(ctx), websocket.StatusInvalidFramePayloadData, "not_json")
				return nil
			}
			result := make(chan handled, 1)
			handling = result
			go s.answerConversationMessage(ctx, exchange, message, result)
		case result := <-handling:
			handling = nil
			ended, err := finishConversationMessage(ctx, exchange.page, result, send)
			if ended {
				return err
			}
		case frame, ok := <-exchange.outgoing:
			if ended, err := forwardFrame(ctx, exchange, frame, ok, send); ended {
				return err
			}
		case <-exchange.page.heartbeat.C:
			if err := exchange.page.send(ctx, &framewire.HeartbeatFrame{}); err != nil {
				return err
			}
		}
	}
}

// forwardFrame sends one conversation frame. When the frames ended it
// closes the page as "lagged" if the subscriber fell behind; ended
// reports that the exchange is over.
func forwardFrame(
	ctx context.Context,
	exchange conversationExchange,
	frame framewire.ServerFrame,
	ok bool,
	send func(framewire.ServerFrame) error,
) (ended bool, err error) {
	if !ok {
		if errors.Is(exchange.frames.Err(), server.ErrLagged) {
			exchange.page.close(context.WithoutCancel(ctx), websocket.StatusCode(4001), "lagged")
		}
		return true, nil
	}
	if err := send(frame); err != nil {
		return true, err
	}
	return false, nil
}

// finishConversationMessage closes invalid input or sends the completed client reply.
func finishConversationMessage(
	ctx context.Context,
	page *pageSocket,
	result handled,
	send func(framewire.ServerFrame) error,
) (bool, error) {
	if errors.Is(result.err, framewire.ErrNotJSON) {
		page.close(context.WithoutCancel(ctx), websocket.StatusInvalidFramePayloadData, "not_json")
		return true, nil
	}
	if result.err != nil {
		return true, result.err
	}
	if result.reply != nil {
		if err := send(result.reply); err != nil {
			return true, err
		}
	}
	return false, nil
}

// answerConversationMessage finishes an admitted client message even if its socket leaves.
func (s *Shard) answerConversationMessage(
	ctx context.Context,
	exchange conversationExchange,
	message pageMessage,
	result chan<- handled,
) {
	reply, err := s.handleMessage(
		context.WithoutCancel(ctx),
		exchange.id,
		exchange.connection,
		exchange.files,
		message.data,
	)
	result <- handled{reply: reply, err: err}
}
