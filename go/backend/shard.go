package backend

import (
	"context"
	"errors"
	"log/slog"
	"runtime/debug"
	"sync"
	"sync/atomic"

	"github.com/wspl/demi/go/gates"
	"github.com/wspl/demi/go/hostremote"
	"github.com/wspl/demi/go/webapi"
)

// ErrClosing answers a call a closing shard does not serve: the backend is
// shutting down and starts no new work.
var ErrClosing = errors.New("the backend is shutting down")

// ErrCallFailed answers a call that panicked; its shard goes on serving.
var ErrCallFailed = errors.New("the shard call failed")

// queueLength is how many calls wait for a shard before their callers wait to
// enqueue more.
const queueLength = 256

// Shards is the way into every user's shard (concurrency.md § The user
// shard). A user's shard is one goroutine, started by the first call to it
// and ended by Close; only that goroutine touches the user's state.
type Shards struct {
	services *Services
	mu       sync.Mutex
	shards   map[webapi.UserID]*Shard
	closed   bool
}

// NewShards starts no shard yet: each starts with its user's first call.
func NewShards(services *Services) *Shards {
	return &Shards{services: services, shards: map[webapi.UserID]*Shard{}}
}

// Of is the way into user's shard.
func (s *Shards) Of(user webapi.UserID) ShardRef {
	return ShardRef{shards: s, user: user}
}

// shard returns user's shard, starting it first; a closed backend starts none.
func (s *Shards) shard(user webapi.UserID) (*Shard, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if shard, ok := s.shards[user]; ok {
		return shard, nil
	}
	if s.closed {
		return nil, ErrClosing
	}
	shard := newShard(user, s.services)
	s.shards[user] = shard
	go shard.serve()
	return shard, nil
}

// Close closes every user's shard, each in the order of backend.md § Startup
// and shutdown and all at once, and answers why the Clouds it did not save
// were not saved. The backend closes its shards once; a later Close finds
// none.
func (s *Shards) Close(ctx context.Context) []string {
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		return nil
	}
	s.closed = true
	shards := make([]*Shard, 0, len(s.shards))
	for _, shard := range s.shards {
		shards = append(shards, shard)
	}
	s.mu.Unlock()
	var mu sync.Mutex
	var failures []string
	var closing sync.WaitGroup
	for _, shard := range shards {
		closing.Go(func() {
			if failure := shard.close(ctx, s.Of(shard.user)); failure != "" {
				mu.Lock()
				failures = append(failures, failure)
				mu.Unlock()
			}
		})
	}
	closing.Wait()
	return failures
}

// ShardRef is the way into one user's shard.
type ShardRef struct {
	shards *Shards
	user   webapi.UserID
}

// Call runs fn on the user's shard, after the calls before it, and returns
// once it has run (g7-backend.md § Shards). fn is an atomic section: nothing
// else of the user runs while it does, so a check and the change it guards
// belong in one call. It must not wait (no runner, provider, tool, Host,
// gate, timer or other shard), and it hands results back through the
// variables it captures, which the caller reads after Call returns.
//
// ctx bounds only the waits to enqueue and to be started: a call the shard
// has started runs to completion, and Call waits for it.
func (r ShardRef) Call(ctx context.Context, fn func(*Shard)) error {
	return r.run(ctx, fn, false)
}

// CallWhileClosing is Call for work the shard still serves while it closes,
// such as a runner's pipe request that the close's own steps may need.
func (r ShardRef) CallWhileClosing(ctx context.Context, fn func(*Shard)) error {
	return r.run(ctx, fn, true)
}

// Executor is Call in the form the packages below the backend take for their
// short state changes (hostremote, agent): a function that runs a closure on
// the shard.
func (r ShardRef) Executor() func(context.Context, func()) error {
	return func(ctx context.Context, fn func()) error {
		return r.Call(ctx, func(*Shard) { fn() })
	}
}

// Adopt moves long work, such as serving an accepted socket, into the user's
// shard: it runs as one of the shard's tasks on a goroutine of its own, with
// a context that ends when the shard closes, and reaches the shard's state
// only through ref. A closing shard refuses it.
func (r ShardRef) Adopt(ctx context.Context, work func(ctx context.Context, ref ShardRef)) error {
	adopted := false
	err := r.Call(ctx, func(s *Shard) {
		adopted = s.tasks.Go(func(ctx context.Context) { work(ctx, r) })
	})
	if err == nil && !adopted {
		return ErrClosing
	}
	return err
}

func (r ShardRef) run(ctx context.Context, fn func(*Shard), whileClosing bool) error {
	shard, err := r.shards.shard(r.user)
	if err != nil {
		return err
	}
	c := &call{fn: fn, whileClosing: whileClosing, done: make(chan error, 1)}
	select {
	case shard.queue <- c:
	case <-shard.ended:
		return ErrClosing
	case <-ctx.Done():
		return ctx.Err()
	}
	select {
	case err := <-c.done:
		return err
	case <-shard.ended:
		// The shard ended with the call still queued, or while its last
		// drain answers it.
		if c.state.CompareAndSwap(callWaiting, callWithdrawn) {
			return ErrClosing
		}
		return <-c.done
	case <-ctx.Done():
		if c.state.CompareAndSwap(callWaiting, callWithdrawn) {
			return ctx.Err()
		}
		// The shard started it: it runs to completion, and what it
		// captured with it.
		return <-c.done
	}
}

// A call's state: waiting in the queue, started by the shard, or withdrawn by
// its caller before it started.
const (
	callWaiting int32 = iota
	callStarted
	callWithdrawn
)

type call struct {
	fn           func(*Shard)
	whileClosing bool
	state        atomic.Int32
	done         chan error
}

// Shard is one user's shard: everything the backend decides for that user
// (concurrency.md § The user shard). Only its goroutine touches it, through
// the calls it serves; its parts join it with the lanes that build them.
type Shard struct {
	user     webapi.UserID
	services *Services
	queue    chan *call
	// ended closes once the shard has closed and serves nothing more.
	ended chan struct{}
	// closingCtx ends when the shard starts closing; every task group's
	// context derives from it.
	closingCtx context.Context
	startClose context.CancelFunc
	// closing is set by the close's first step: from then on only
	// CallWhileClosing is served.
	closing bool

	// Every goroutine the shard owns; its close waits for them.
	tasks *TaskGroup
	// The conversation sockets being served, which the close ends before the
	// agent shuts down, so no frame reaches it after.
	conversationSockets *TaskGroup
	// The pages' synchronization channels being served, which the close ends
	// first, so no page is sent what the steps after it change.
	syncChannels *TaskGroup

	conversations *Conversations
	titles        *Titles
	// Fork requests, one at a time per destination id in lower case.
	forks       *gates.KeyedSerialGate[string]
	agent       *ConversationAgent
	devices     *Devices
	pipes       *hostremote.Pipes
	commands    *CommandRouter
	cloud       *Cloud
	idleWatches *ConversationWatches
	claudeCLI   *ClaudeCLI
	exposes     *Exposes
}

func newShard(user webapi.UserID, services *Services) *Shard {
	closingCtx, startClose := context.WithCancel(context.Background())
	return &Shard{
		user:                user,
		services:            services,
		queue:               make(chan *call, queueLength),
		ended:               make(chan struct{}),
		closingCtx:          closingCtx,
		startClose:          startClose,
		tasks:               newTaskGroup(closingCtx),
		conversationSockets: newTaskGroup(closingCtx),
		syncChannels:        newTaskGroup(closingCtx),
		conversations:       &Conversations{},
		titles:              &Titles{},
		forks:               gates.NewKeyedSerialGate[string](),
		agent:               &ConversationAgent{},
		devices:             &Devices{},
		pipes:               hostremote.NewPipes(hostremote.Arrival),
		commands:            &CommandRouter{},
		cloud:               &Cloud{},
		idleWatches:         &ConversationWatches{},
		claudeCLI:           &ClaudeCLI{},
		exposes:             &Exposes{},
	}
}

// User is the user whose shard this is.
func (s *Shard) User() webapi.UserID { return s.user }

// Services are what the users share.
func (s *Shard) Services() *Services { return s.services }

// Closing reports whether the shard has started closing and starts no new
// work.
func (s *Shard) Closing() bool { return s.closing }

// Tasks are the goroutines the shard owns; its close waits for them.
func (s *Shard) Tasks() *TaskGroup { return s.tasks }

// ConversationSockets are the conversation sockets being served.
func (s *Shard) ConversationSockets() *TaskGroup { return s.conversationSockets }

// SyncChannels are the pages' synchronization channels being served.
func (s *Shard) SyncChannels() *TaskGroup { return s.syncChannels }

// Conversations are the user's conversations.
func (s *Shard) Conversations() *Conversations { return s.conversations }

// Titles are the title requests of the user's conversations.
func (s *Shard) Titles() *Titles { return s.titles }

// Forks admit fork requests one at a time per destination id in lower case.
func (s *Shard) Forks() *gates.KeyedSerialGate[string] { return s.forks }

// Agent is the user's conversation trees.
func (s *Shard) Agent() *ConversationAgent { return s.agent }

// Devices are the user's devices with their runner connections.
func (s *Shard) Devices() *Devices { return s.devices }

// Pipes are the pipes of the user's devices.
func (s *Shard) Pipes() *hostremote.Pipes { return s.pipes }

// Commands route each agent node's commands to the rpc calls of its jobs.
func (s *Shard) Commands() *CommandRouter { return s.commands }

// Cloud is the user's Cloud machine.
func (s *Shard) Cloud() *Cloud { return s.cloud }

// IdleWatches are each conversation's idle watch.
func (s *Shard) IdleWatches() *ConversationWatches { return s.idleWatches }

// ClaudeCLI is the Claude Code CLI work on the user's Cloud.
func (s *Shard) ClaudeCLI() *ClaudeCLI { return s.claudeCLI }

// Exposes are the user's exposes with relayed connections open.
func (s *Shard) Exposes() *Exposes { return s.exposes }

// serve runs the shard's calls one at a time until the shard has closed, and
// then refuses those still queued.
func (s *Shard) serve() {
	for {
		select {
		case c := <-s.queue:
			s.run(c)
		case <-s.ended:
			for {
				select {
				case c := <-s.queue:
					if c.state.CompareAndSwap(callWaiting, callStarted) {
						c.done <- ErrClosing
					}
				default:
					return
				}
			}
		}
	}
}

func (s *Shard) run(c *call) {
	if !c.state.CompareAndSwap(callWaiting, callStarted) {
		return
	}
	if s.closing && !c.whileClosing {
		c.done <- ErrClosing
		return
	}
	c.done <- s.guarded(c.fn)
}

// guarded runs fn, turning its panic into ErrCallFailed so the shard goes on
// serving; the panic is logged with its stack.
func (s *Shard) guarded(fn func(*Shard)) (err error) {
	defer func() {
		if value := recover(); value != nil {
			slog.Error("a shard call panicked", "user", s.user.String(), "panic", value, "stack", string(debug.Stack()))
			err = ErrCallFailed
		}
	}()
	fn(s)
	return nil
}

// close ends the user's work in the order of backend.md § Startup and
// shutdown, from a goroutine of its own: it changes the shard's state through
// ref's CallWhileClosing, which the shard serves meanwhile, and waits between
// the calls. It answers why the Cloud was not saved, or "" if it was.
func (s *Shard) close(ctx context.Context, ref ShardRef) string {
	step := func(fn func(*Shard)) {
		// A call on the shard's own ref fails only for a shard that has
		// already ended, which no close step outlives.
		_ = ref.CallWhileClosing(context.WithoutCancel(ctx), fn)
	}
	step(func(s *Shard) {
		s.closing = true
		s.startClose()
		s.syncChannels.close()
	})
	s.syncChannels.wait()
	step(func(s *Shard) {
		s.idleWatches.stopAll()
		s.cloud.stop()
		s.titles.abortAll()
		s.exposes.endAll()
		s.conversationSockets.close()
	})
	s.conversationSockets.wait()
	endTransfers(ctx, ref)
	shutDownAgent(ctx, ref)
	failure := saveCloud(ctx, ref)
	step(func(s *Shard) {
		s.devices.disconnectAll("backend shutting down")
		s.tasks.close()
	})
	s.tasks.wait()
	// The pipes fail every waiting end; their close error has no one to tell
	// but the log.
	if err := s.pipes.Close(); err != nil {
		slog.Warn("closing a shard's pipes", "user", s.user.String(), "error", err)
	}
	close(s.ended)
	return failure
}
