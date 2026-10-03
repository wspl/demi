package usershard

import (
	"context"
	"slices"
	"sync"
	"time"

	"github.com/coder/websocket"

	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

func (s *Shard) serveSync(ctx context.Context, socket *websocket.Conn, session ChannelSession) error {
	// Closing an already broken socket needs no recovery.
	defer func() { _ = socket.CloseNow() }()
	_, err := shardCall(ctx, s, func(ctx context.Context) (struct{}, error) { return struct{}{}, s.relaySync(ctx, socket, session) })
	return err
}
func (s *Shard) relaySync(ctx context.Context, socket *websocket.Conn, session ChannelSession) (err error) {
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
	incoming := readPage(readCtx, socket, &workers)
	// The reader ends a stalled state read or write as soon as the page speaks.
	ended := make(chan pageMessage, 1)
	workers.Add(1)
	go func() {
		defer workers.Done()
		select {
		case message := <-incoming:
			ended <- message
			cancel()
		case <-ctx.Done():
		}
	}()
	closeCode, reason := websocket.StatusInternalError, "internal_error"
	defer func() {
		if s.ctx.Err() != nil {
			closeCode, reason = websocket.StatusGoingAway, "backend_closing"
		} else {
			select {
			case message := <-ended:
				if message.err != nil {
					return
				}
				closeCode, reason = websocket.StatusPolicyViolation, "unexpected_message"
			default:
			}
		}
		page.close(context.WithoutCancel(ctx), closeCode, reason)
	}()
	state, err := s.productState(ctx, session.User)
	if err != nil {
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
	followers := make([]pagesync.Part, 0)
	for _, id := range s.services.Plugins.Followers(plugin.TopicExposes) {
		followers = append(followers, pagesync.Part{Kind: pagesync.Plugin, PluginID: string(id)})
	}
	expiry := time.NewTimer(time.Hour)
	expiry.Stop()
	defer expiry.Stop()
	var expires <-chan time.Time
	readExpiry := func() error {
		if len(followers) == 0 {
			return nil
		}
		listed, err := expose.List(ctx, s.ExposeShard())
		if err != nil {
			return err
		}
		expires = nil
		expiry.Stop()
		if len(listed) == 0 {
			return nil
		}
		first := listed[0].Record.ExpiresAt
		for _, entry := range listed {
			if entry.Record.ExpiresAt < first {
				first = entry.Record.ExpiresAt
			}
		}
		at, err := first.Time()
		if err != nil {
			return err
		}
		now, err := s.Clock().Now().Time()
		if err != nil {
			return err
		}
		expiry.Reset(max(0, at.Sub(now)))
		expires = expiry.C
		return nil
	}
	if err := readExpiry(); err != nil {
		return err
	}
	marked := make(chan struct{})
	taken := make(chan struct{})
	workers.Add(1)
	go func() {
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
	}()
	for {
		var parts []pagesync.Part
		heartbeat := false
		marks := false
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-marked:
			marks = true
		case <-expires:
			parts = followers
		case <-page.heartbeat.C:
			heartbeat = true
		}
		if s.Clock().Now() >= session.ExpiresAt {
			current, err := s.services.Sessions.Check(ctx, session.Token)
			if err != nil {
				return err
			}
			if current == nil {
				closeCode, reason = websocket.StatusCode(4002), "session_ended"
				return nil
			}
			session.ExpiresAt = current.ExpiresAt
		}
		if marks {
			if s.services.Hooks != nil {
				if err := s.services.Hooks.Sync(ctx, SyncChanges); err != nil {
					return err
				}
			}
			changes := registration.Take()
			select {
			case taken <- struct{}{}:
			case <-ctx.Done():
				return ctx.Err()
			}
			if changes.SessionEnded {
				closeCode, reason = websocket.StatusCode(4002), "session_ended"
				return nil
			}
			parts = changes.Parts
		}
		if heartbeat {
			if err := page.send(ctx, &webapi.SyncEventHeartbeat{}); err != nil {
				return err
			}
		}
		for _, part := range parts {
			event, err := s.readPart(ctx, part, session.User)
			if err != nil {
				return err
			}
			if event == nil {
				continue
			}
			if user, ok := event.(*webapi.SyncEventUser); ok {
				session.User = user.User
			}
			if err := page.send(ctx, event); err != nil {
				return err
			}
			if slices.Contains(followers, part) {
				if err := readExpiry(); err != nil {
					return err
				}
			}
		}
	}
}
