package usershard

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"sync"
	"time"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/expose"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/plugins"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/cmdpkg/claudecode/claudecodeop"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/webapi"
)

// Shard owns one user's runtime state. One mutex protects short decisions;
// no IO, callback, channel operation or join runs while it is held. Methods
// accept concurrent calls; returned component handles synchronize themselves.
type Shard struct {
	mu            sync.Mutex
	user          webapi.UserID
	services      *Services
	http          *http.Client
	ctx           context.Context
	cancel        context.CancelFunc
	closing       bool
	calls         sync.WaitGroup
	saves         sync.WaitGroup
	work          sync.WaitGroup
	runnerCtx     context.Context
	runnerCancel  context.CancelFunc
	runners       sync.WaitGroup
	runnerOrder   gates.KeyedSerial[webapi.DeviceID]
	cloud         *cloud.Cloud
	devices       runners.Devices
	pipes         *remotehost.Pipes
	commands      runners.CommandRouter
	conversations *hostaccess.Conversations
	installs      *hostaccess.PluginInstalls
	plugins       *plugins.User
	agent         *server.Server[*remotehost.Host]
	providers     *conversationProviders
	exposes       expose.Exposes
	deviceOrder   gates.KeyedSerial[webapi.DeviceID]
	forks         gates.KeyedSerial[string]
	titles        map[webapi.ConversationID]*idleWatch
	idle          map[webapi.ConversationID]*idleWatch
	upgrading     map[claudecodeop.Version]bool
	jobsEnded     map[webapi.ConversationID]uint64
}

// Shards routes each user to its unique in-process shard and owns every shard's
// workers and sockets. Close refuses new work and joins work already admitted.
type Shards struct {
	mu        sync.Mutex
	services  *Services
	ctx       context.Context
	users     map[webapi.UserID]*Shard
	phase     routingPhase
	closeOnce sync.Once
	closeDone chan struct{}
	closeErr  error
}

type routingPhase uint8

const (
	routingOpen routingPhase = iota
	routingDraining
	routingClosed
)

// NewShards creates routing over services. ctx is the backend lifetime, not a
// request lifetime. The owner must Close before disposing services.
func NewShards(ctx context.Context, services *Services) (*Shards, error) {
	return &Shards{
		services:  services,
		ctx:       ctx,
		users:     make(map[webapi.UserID]*Shard),
		closeDone: make(chan struct{}),
	}, nil
}

// Of returns the user's stable shard, creating it on first use. Routing refuses
// new requests once shutdown starts; callers invoke shard methods directly.
func (s *Shards) Of(ctx context.Context, user webapi.UserID) (*Shard, error) {
	return s.acquireShard(ctx, user, false)
}

// OfWhileClosing returns the user's shard for runner pipe requests needed by
// shutdown. It remains available during draining and refuses after routing has
// fully closed. Ordinary product requests use Of.
func (s *Shards) OfWhileClosing(ctx context.Context, user webapi.UserID) (*Shard, error) {
	return s.acquireShard(ctx, user, true)
}

// Close drains admitted transitions and sockets and joins all shard workers.
func (s *Shards) Close(ctx context.Context) error {
	s.closeOnce.Do(func() {
		s.mu.Lock()
		s.phase = routingDraining
		users := make([]*Shard, 0, len(s.users))
		for _, shard := range s.users {
			users = append(users, shard)
		}
		s.mu.Unlock()
		for _, shard := range users {
			shard.beginClose()
		}
		var failures []error
		for _, shard := range users {
			err := shard.close(context.WithoutCancel(ctx))
			failures = append(failures, err)
		}
		s.mu.Lock()
		s.phase = routingClosed
		s.closeErr = errors.Join(failures...)
		s.mu.Unlock()
		close(s.closeDone)
	})
	<-s.closeDone
	return s.closeErr
}

// Services returns the immutable shared service handles.
func (s *Shard) Services() *Services { return s.services }

// HTTP returns the shard's provider HTTP client.
func (s *Shard) HTTP() *http.Client { return s.http }

// Plugins returns the synchronized plugin host of this user.
func (s *Shard) Plugins() *plugins.User { return s.plugins }

// Closed is closed when shutdown begins; it does not signal that draining ended.
func (s *Shard) Closed() <-chan struct{} { return s.ctx.Done() }

// HostShard supplies the conversation Host access boundary.
func (s *Shard) HostShard() hostaccess.HostShard { return s }

// ExposeShard supplies the expose boundary through a private adapter because
// its Control, PublicURL and Exposes signatures differ from other consumers.
func (s *Shard) ExposeShard() expose.ExposeShard { return exposeView{s} }

// RouteDeaths routes manager death events to the owning user's Cloud until
// deaths closes or ctx ends. Its caller owns and joins the call.
func RouteDeaths(
	ctx context.Context,
	deaths <-chan webapi.DeviceID,
	services *Services,
	shards *Shards,
) error {
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case device, ok := <-deaths:
			if !ok {
				return nil
			}
			record, err := services.Control.Device(ctx, device)
			if err != nil {
				slog.ErrorContext(ctx, "the death of a Cloud could not be routed", "device", device, "error", err)
				continue
			}
			if record == nil {
				continue
			}
			shard, err := shards.Of(ctx, record.User)
			if err != nil {
				continue
			}
			if err := cloud.Died(ctx, shard, device); err != nil {
				slog.WarnContext(ctx, "the Cloud death was not handled", "device", device, "error", err)
			}
		}
	}
}

// ScheduleRetention runs the first retention pass immediately and subsequent
// passes at the configured interval. Its caller owns and joins the call.
func ScheduleRetention(ctx context.Context, services *Services, shards *Shards) error {
	interval := services.Lifecycle.RetentionInterval
	if interval == nil {
		return nil
	}
	for {
		users, err := services.Control.Users(ctx)
		if err != nil {
			slog.ErrorContext(ctx, "the retention pass cannot list the users", "error", err)
		}
		for _, user := range users {
			shard, err := shards.Of(ctx, user.ID)
			if err != nil {
				return err
			}
			if err := shard.RetentionPass(ctx); err != nil {
				slog.WarnContext(ctx, "the retention pass failed", "user", user.ID, "error", err)
			}
		}
		timer := time.NewTimer(*interval)
		select {
		case <-ctx.Done():
			timer.Stop()
			return ctx.Err()
		case <-timer.C:
		}
	}
}

// RetentionPass collects this user's retained conversation and blob data.
func (s *Shard) RetentionPass(ctx context.Context) error {
	_, err := shardCall(
		ctx,
		s,
		func(ctx context.Context) (struct{}, error) { return struct{}{}, s.removeExpiredConversations(ctx) },
	)
	return err
}

// RecoverForks publishes reserved destinations whose roots committed; other
// destinations remain hidden for retry. Call before serving requests.
func RecoverForks(
	ctx context.Context,
	control *database.ControlService,
	conversations *database.ConversationStores,
) error {
	pending, err := control.PendingForks(ctx)
	if err != nil {
		return err
	}
	for _, operation := range pending {
		committed, err := forkCommitted(ctx, conversations, operation.ID)
		if err != nil {
			return err
		}
		if committed {
			if _, err := control.PublishFork(ctx, operation.ID); err != nil {
				return err
			}
		}
	}
	return nil
}

// RearmWakeups reopens roots with pending persisted wakeups after startup.
func RearmWakeups(ctx context.Context, control *database.ControlService, shards *Shards) error {
	saved, err := control.SavedWakeups(ctx)
	if err != nil {
		return err
	}
	for _, wakeup := range saved {
		shard, err := shards.Of(ctx, wakeup.Owner)
		if err != nil {
			return nil
		}
		shard.startWorker(func(ctx context.Context) {
			shard.restoreWhenDue(ctx, wakeup.Conversation, wakeup.Due)
		})
	}
	return nil
}

// of selects the user's shard without introducing a per-request mailbox.
func (s *Shards) acquireShard(ctx context.Context, user webapi.UserID, draining bool) (*Shard, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	drainingRefused := s.phase == routingDraining && !draining
	if s.phase == routingClosed || drainingRefused {
		return nil, &ShardUnavailable{Kind: ShardClosing}
	}
	if shard := s.users[user]; shard != nil {
		return shard, nil
	}
	if s.phase == routingDraining {
		return nil, &ShardUnavailable{Kind: ShardClosing}
	}
	shard := newShard(s.ctx, user, s.services)
	s.users[user] = shard
	return shard, nil
}

// newShard composes the services of one user; its components load data on demand.
func newShard(ctx context.Context, user webapi.UserID, services *Services) *Shard {
	ctx, cancel := context.WithCancel(ctx)
	s := &Shard{
		user:      user,
		services:  services,
		http:      &http.Client{},
		ctx:       ctx,
		cancel:    cancel,
		jobsEnded: make(map[webapi.ConversationID]uint64),
	}
	s.runnerCtx, s.runnerCancel = context.WithCancel(context.WithoutCancel(ctx))
	s.cloud = cloud.New(ctx, &s.mu)
	s.conversations = hostaccess.NewConversations(ctx, &s.mu)
	s.installs = hostaccess.NewPluginInstalls(&s.mu)
	s.pipes = remotehost.NewPipes(remotehost.Arrival)
	s.plugins = plugins.NewUser(services.Plugins, user, s)
	s.composeAgent()
	return s
}

// shardCall owns a product operation until it finishes, including commit
// bookkeeping after request cancellation, and contains panics at the boundary.
func shardCall[T any](
	ctx context.Context,
	s *Shard,
	operation func(context.Context) (T, error),
) (result T, err error) {
	s.mu.Lock()
	if s.closing {
		s.mu.Unlock()
		return result, &ShardUnavailable{Kind: ShardClosing}
	}
	s.calls.Add(1)
	s.mu.Unlock()
	defer s.calls.Done()
	defer func() {
		if failure := recover(); failure != nil {
			err = &ShardUnavailable{Kind: ShardFailed, Err: fmt.Errorf("%v", failure)}
		}
	}()
	return operation(ctx)
}

func (s *Shard) beginClose() {
	s.mu.Lock()
	s.closing = true
	s.mu.Unlock()
	s.cancel()
	s.exposes.EndAll()
}

func (s *Shard) drainCalls() {
	s.beginClose()
	s.calls.Wait()
}

func (s *Shard) close(ctx context.Context) error {
	s.drainCalls()
	s.work.Wait()
	var failures []error
	holds, err := s.conversations.EndTransfers(ctx)
	failures = append(failures, err)
	for _, hold := range holds {
		defer hold.Release()
	}
	failures = append(failures, s.exposes.Close(ctx))
	if s.agent != nil {
		failures = append(failures, s.agent.Shutdown(ctx))
	}
	s.saves.Wait()
	failures = append(failures, cloud.Close(ctx, s))
	s.devices.DisconnectAll("the backend is shutting down")
	s.runnerCancel()
	s.runners.Wait()
	failures = append(failures, s.conversations.Close(ctx), s.plugins.Close(ctx), s.pipes.Close(ctx))
	s.http.CloseIdleConnections()
	return errors.Join(failures...)
}
