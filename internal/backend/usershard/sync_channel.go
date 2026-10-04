package usershard

import (
	"context"
	"log/slog"
	"slices"
	"sync"
	"time"

	"github.com/coder/websocket"

	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

func (s *Shard) serveSyncChannel(ctx context.Context, socket *PageConnection, session ChannelSession) error {
	// Closing an already broken socket needs no recovery.
	defer func() {
		_ = socket.CloseNow()
	}()
	_, err := shardCall(
		ctx,
		s,
		func(ctx context.Context) (struct{}, error) { return struct{}{}, s.relaySync(ctx, socket, session) },
	)
	return err
}

func (s *Shard) relaySync(ctx context.Context, socket *PageConnection, session ChannelSession) (err error) {
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
	page := newPageSocket(socket, s.services.Pages)
	defer page.heartbeat.Stop()
	registration := s.services.Sync.Register(s.user, session.Token)
	defer registration.Release()
	var workers sync.WaitGroup
	defer func() {
		cancel()
		_ = socket.CloseNow()
		workers.Wait()
	}()
	readCtx, readCancel := context.WithCancel(context.WithoutCancel(ctx))
	defer readCancel()
	ended := startSyncReader(ctx, readCtx, socket.Conn, cancel, &workers)
	closeCode, reason := websocket.StatusInternalError, "internal_error"
	defer s.closeSyncPage(ctx, page, ended, &closeCode, &reason)
	if err := s.sendSyncSnapshot(ctx, page, session.User); err != nil {
		return err
	}
	followers := s.exposeFollowers()
	expiry := time.NewTimer(time.Hour)
	expiry.Stop()
	defer expiry.Stop()
	var expires <-chan time.Time
	readExpiry := func() error {
		var err error
		expires, err = s.exposeExpiry(ctx, followers, expiry)
		return err
	}
	if err := readExpiry(); err != nil {
		return err
	}
	marked := make(chan struct{})
	taken := make(chan struct{})
	workers.Add(1)
	go forwardSyncMarks(ctx, registration, marked, taken, &workers)
	return s.exchangeSync(ctx, syncExchange{
		page:         page,
		session:      &session,
		registration: registration,
		followers:    followers,
		expires:      &expires,
		readExpiry:   readExpiry,
		marked:       marked,
		taken:        taken,
		closeCode:    &closeCode,
		reason:       &reason,
	})
}

// exposeExpiry arms the timer for the earliest expose followed by the page.
func (s *Shard) exposeExpiry(
	ctx context.Context,
	followers []pagesync.Part,
	expiry *time.Timer,
) (<-chan time.Time, error) {
	if len(followers) == 0 {
		return nil, nil
	}
	listed, err := expose.List(ctx, s.ExposeShard())
	if err != nil {
		slog.ErrorContext(ctx, "a page's exposes could not be read", "error", err)
		return nil, err
	}
	expiry.Stop()
	if len(listed) == 0 {
		return nil, nil
	}
	first := listed[0].Record.ExpiresAt
	for _, entry := range listed {
		if entry.Record.ExpiresAt < first {
			first = entry.Record.ExpiresAt
		}
	}
	at, err := first.Time()
	if err != nil {
		return nil, err
	}
	now, err := s.Clock().Now().Time()
	if err != nil {
		return nil, err
	}
	delay := max(0, at.Sub(now))
	expiry.Reset(delay)
	return expiry.C, nil
}

// forwardSyncMarks waits for the page to take each notification before forwarding another.
func forwardSyncMarks(
	ctx context.Context,
	registration *pagesync.Registration,
	marked chan<- struct{},
	taken <-chan struct{},
	workers *sync.WaitGroup,
) {
	defer workers.Done()
	for {
		if registration.Marked(ctx) != nil {
			return
		}
		select {
		case marked <- struct{}{}:
		case <-ctx.Done():
			return
		}
		select {
		case <-taken:
		case <-ctx.Done():
			return
		}
	}
}

type syncExchange struct {
	page         *pageSocket
	session      *ChannelSession
	registration *pagesync.Registration
	followers    []pagesync.Part
	expires      *<-chan time.Time
	readExpiry   func() error
	marked       <-chan struct{}
	taken        chan<- struct{}
	closeCode    *websocket.StatusCode
	reason       *string
}

// exchangeSync sends changed page parts and checks whether the session has ended.
func (s *Shard) exchangeSync(ctx context.Context, exchange syncExchange) error {
	for {
		var parts []pagesync.Part
		heartbeat := false
		marks := false
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-exchange.marked:
			marks = true
		case <-*exchange.expires:
			parts = exchange.followers
		case <-exchange.page.heartbeat.C:
			heartbeat = true
		}
		if s.Clock().Now() >= exchange.session.ExpiresAt {
			current, found, err := s.services.Sessions.Check(ctx, exchange.session.Token)
			if err != nil {
				slog.ErrorContext(ctx, "a page's session could not be read", "error", err)
				return err
			}
			if !found {
				*exchange.closeCode, *exchange.reason = websocket.StatusCode(4002), "session_ended"
				return nil
			}
			exchange.session.ExpiresAt = current.ExpiresAt
		}
		if marks {
			if err := s.syncHook(ctx, SyncChanges); err != nil {
				return err
			}
			changes := exchange.registration.Take()
			select {
			case exchange.taken <- struct{}{}:
			case <-ctx.Done():
				return ctx.Err()
			}
			if changes.SessionEnded {
				*exchange.closeCode, *exchange.reason = websocket.StatusCode(4002), "session_ended"
				return nil
			}
			parts = changes.Parts
		}
		if heartbeat {
			if err := exchange.page.send(ctx, &webapi.SyncEventHeartbeat{}); err != nil {
				return err
			}
		}
		if err := s.sendSyncParts(ctx, exchange, parts); err != nil {
			return err
		}
	}
}

// syncHook waits at an optional page synchronization point.
func (s *Shard) syncHook(ctx context.Context, step SyncStep) error {
	if s.services.Hooks == nil {
		return nil
	}
	return s.services.Hooks.Sync(ctx, step)
}

// sendSyncParts presents changed parts and rechecks expose expiry after relevant changes.
func (s *Shard) sendSyncParts(ctx context.Context, exchange syncExchange, parts []pagesync.Part) error {
	for _, part := range parts {
		event, err := s.readPart(ctx, part, exchange.session.User)
		if err != nil {
			slog.ErrorContext(ctx, "a part of a page's state could not be read", "error", err)
			return err
		}
		if event == nil {
			continue
		}
		if user, ok := event.(*webapi.SyncEventUser); ok {
			exchange.session.User = user.User
		}
		if err := exchange.page.send(ctx, event); err != nil {
			return err
		}
		if slices.Contains(exchange.followers, part) {
			if err := exchange.readExpiry(); err != nil {
				return err
			}
		}
	}
	return nil
}

// closeSyncPage preserves backend shutdown and unexpected-message close reasons.
func (s *Shard) closeSyncPage(
	ctx context.Context,
	page *pageSocket,
	ended <-chan pageMessage,
	closeCode *websocket.StatusCode,
	reason *string,
) {
	if s.ctx.Err() != nil {
		*closeCode, *reason = websocket.StatusGoingAway, "backend_closing"
	} else {
		select {
		case message := <-ended:
			if message.err != nil {
				return
			}
			*closeCode, *reason = websocket.StatusPolicyViolation, "unexpected_message"
		default:
		}
	}
	page.close(context.WithoutCancel(ctx), *closeCode, *reason)
}

// sendSyncSnapshot reads and sends initial product state after the snapshot hook.
func (s *Shard) sendSyncSnapshot(ctx context.Context, page *pageSocket, user webapi.UserDTO) error {
	state, err := s.productState(ctx, user)
	if err != nil {
		slog.ErrorContext(ctx, "a page's product state could not be read", "error", err)
		return err
	}
	if s.services.Hooks != nil {
		if err := s.services.Hooks.Sync(ctx, SyncSnapshot); err != nil {
			return err
		}
	}
	if err := page.send(ctx, &webapi.SyncEventSnapshot{State: state}); err != nil {
		return err
	}
	return nil
}

// watchSyncInput cancels stalled page work as soon as the page sends a message or disconnects.
func watchSyncInput(
	ctx context.Context,
	incoming <-chan pageMessage,
	ended chan<- pageMessage,
	cancel context.CancelFunc,
	workers *sync.WaitGroup,
) {
	defer workers.Done()
	select {
	case message := <-incoming:
		ended <- message
		cancel()
	case <-ctx.Done():
	}
}

// exposeFollowers lists page state parts that follow expose expiry.
func (s *Shard) exposeFollowers() []pagesync.Part {
	followers := make([]pagesync.Part, 0)
	for _, id := range s.services.Plugins.Followers(plugin.TopicExposes) {
		followers = append(followers, pagesync.Part{Kind: pagesync.Plugin, PluginID: string(id)})
	}
	return followers
}

// startSyncReader watches page input so it can cancel stalled state work.
func startSyncReader(
	ctx context.Context,
	readCtx context.Context,
	socket *websocket.Conn,
	cancel context.CancelFunc,
	workers *sync.WaitGroup,
) <-chan pageMessage {
	incoming := readPage(readCtx, socket, workers)
	// The reader ends a stalled state read or write as soon as the page speaks.
	ended := make(chan pageMessage, 1)
	workers.Add(1)
	go watchSyncInput(ctx, incoming, ended, cancel, workers)
	return ended
}
