package usershard

import (
	"context"
	"fmt"
	"log/slog"
	"reflect"
	"time"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/tools"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/backend/pluginhost"
	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

const instructions = "You are a coding agent. Use shell session tools to inspect, edit, test, " +
	"and verify the workspace.\n\nTreat cwd as the task workspace. Create, edit," +
	" and verify task files there by default; do not create a separate project " +
	"directory under /tmp or another absolute path unless the user asks for it " +
	"or the workspace is unusable."

type shardToolsets struct{ shard *Shard }

// Current returns the enabled plugin commands, profiles and revision.
func (t shardToolsets) Current(ctx context.Context) (tools.Set, error) {
	set, err := t.shard.plugins.Toolset(ctx, []host.Declared{hostaccess.HostGroup(t.shard)})
	return tools.Set{
		Commands: set.Commands,
		Profiles: set.Profiles,
		Revision: set.Revision,
	}, err
}

type shardHosts struct{ shard *Shard }

// Host resolves the node’s host through conversation host access.
func (h shardHosts) Host(ctx context.Context, node tools.NodeContext) (*remotehost.Host, error) {
	return hostaccess.ConversationHostForNode(ctx, h.shard, hostaccess.ConversationOf(node.Root))
}

type executionContext struct{ shard *Shard }

// Name identifies this context source’s transcript blocks.
func (executionContext) Name() string {
	return plugin.ExecutionSource
}

// Context returns source text the node has not yet observed.
func (e executionContext) Context(
	ctx context.Context,
	node tools.NodeContext,
	_ types.TurnID,
	seen []string,
) (*string, error) {
	text, err := e.shard.executionContext(ctx, hostaccess.ConversationOf(node.Root), seen)
	if err != nil {
		return nil, fmt.Errorf("the execution context cannot be read: %w", err)
	}
	return text, nil
}

type pluginContext struct {
	shard  *Shard
	plugin plugin.ID
}

// Name identifies this context source’s transcript blocks.
func (p pluginContext) Name() string {
	return string(p.plugin)
}

// Context returns source text the node has not yet observed.
func (p pluginContext) Context(
	ctx context.Context,
	node tools.NodeContext,
	turn types.TurnID,
	seen []string,
) (*string, error) {
	text, err := p.shard.plugins.Context(
		ctx,
		p.plugin,
		pluginhost.ContextAsk{
			Conversation: hostaccess.ConversationOf(node.Root),
			Node:         node.Node,
			Cwd:          node.CWD,
			Turn:         turn,
			Seen:         seen,
		},
	)
	if err != nil {
		return nil, fmt.Errorf("the plugin %s gave no context: %w", p.plugin, err)
	}
	return text, nil
}

func (s *Shard) composeAgent() {
	s.providers = &conversationProviders{
		shard: s,
		rate:  providerhost.NewRequestRateLimit(s.services.ConversationTuning.RequestsPerMinute),
	}
	sources := []tools.ContextSource{executionContext{shard: s}}
	for _, id := range s.services.Plugins.ContextSources() {
		sources = append(sources, pluginContext{shard: s, plugin: id})
	}
	config := server.DefaultConfig()
	config.OutboxFrames = s.services.ConversationTuning.OutboxFrames
	s.agent = server.New(server.Deps[*remotehost.Host]{
		Toolsets: shardToolsets{
			shard: s,
		},
		Instructions: instructions,
		Hosts:        shardHosts{shard: s},
		Context:      sources,
		Providers:    s.providers,
		Shells:       hostaccess.NewShardShellEnvironments(s, s.services.Native.Catalog(s.services.PublicURL)),
		Stores: func(root types.NodeID) store.Tree {
			id := hostaccess.ConversationOf(root)
			indexed := &indexedWakeup{shard: s, conversation: id}
			return database.NewTreeStore(
				s.services.Conversations.DB(id),
				s.BlobUses(),
				func(node types.NodeID, due database.WakeupDue) {
					if node == root {
						s.Mark(pagesync.Part{Kind: pagesync.Conversation, ConversationID: id})
					}
					indexed.committed(due)
				},
			)
		},
		Clock:  s.Clock(),
		IDs:    transcript.RandomIDs{},
		Config: config,
		StatusChanged: func(root types.NodeID) {
			id := hostaccess.ConversationOf(root)
			s.Mark(pagesync.Part{Kind: pagesync.Conversation, ConversationID: id})
			if s.agent.Tree(root) == nil {
				disposed := s.Clock().Now()
				s.mu.Lock()
				s.saves.Add(1)
				s.mu.Unlock()
				go func() {
					defer s.saves.Done()
					if err := s.Control().MarkLive(context.WithoutCancel(s.ctx), id, disposed); err != nil {
						slog.Warn("the disposal was not recorded", "conversation", id, "error", err)
					}
					s.startWorker(func(ctx context.Context) {
						if err := s.retireMedia(ctx, id, true); err != nil {
							slog.WarnContext(ctx, "tool media not retired", "conversation", id, "error", err)
						}
					})
				}()
			}
		},
	})
}

// startWorker registers a shard worker before publishing it to shutdown.
func (s *Shard) startWorker(run func(context.Context)) bool {
	s.mu.Lock()
	if s.closing {
		s.mu.Unlock()
		return false
	}
	s.work.Add(1)
	s.mu.Unlock()
	go func() {
		defer s.work.Done()
		run(s.ctx)
	}()
	return true
}

// indexedWakeup coalesces a tree store's saved wakeups in commit order.
// Its state shares the shard mutex; its database writes run outside it.
type indexedWakeup struct {
	shard        *Shard
	conversation webapiproto.ConversationID
	latest       database.WakeupDue
	indexed      database.WakeupDue
	known        bool
	writing      bool
}

func (w *indexedWakeup) committed(due database.WakeupDue) {
	s := w.shard
	s.mu.Lock()
	w.latest = due
	alreadyIndexed := false
	if !w.writing {
		alreadyIndexed = w.known && reflect.DeepEqual(w.indexed, due)
	}
	if w.writing || alreadyIndexed {
		s.mu.Unlock()
		return
	}
	w.writing = true
	s.saves.Add(1)
	s.mu.Unlock()
	go func() {
		defer s.saves.Done()
		for {
			s.mu.Lock()
			next := w.latest
			if w.known && reflect.DeepEqual(w.indexed, next) {
				w.writing = false
				s.mu.Unlock()
				return
			}
			s.mu.Unlock()
			err := s.Control().SetWakeup(context.WithoutCancel(s.ctx), w.conversation, next)
			s.mu.Lock()
			if err != nil {
				w.writing = false
				s.mu.Unlock()
				slog.Warn(
					"the conversation's saved wakeup was not indexed",
					"conversation",
					w.conversation,
					"error",
					err,
				)
				return
			}
			w.known = true
			w.indexed = next
			s.mu.Unlock()
		}
	}()
}

func (s *Shard) restoreWhenDue(
	ctx context.Context,
	id webapiproto.ConversationID,
	due database.WakeupDue,
) {
	if at, ok := due.(*database.WakeupAt); ok {
		when, err := at.At.Time()
		if err != nil {
			slog.Warn("the saved wakeup time cannot be read", "error", err)
			return
		}
		now, err := s.Clock().Now().Time()
		if err != nil {
			slog.Warn("the clock cannot be read", "error", err)
			return
		}
		delay := max(0, when.Sub(now))
		timer := time.NewTimer(delay)
		defer timer.Stop()
		select {
		case <-ctx.Done():
			return
		case <-timer.C:
		}
	}
	if ctx.Err() != nil {
		return
	}
	if err := s.restoreTree(ctx, id); err != nil {
		slog.Warn("the tree of a saved wakeup was not restored", "conversation", id, "error", err)
	}
}
