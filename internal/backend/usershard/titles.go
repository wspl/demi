package usershard

import (
	"context"
	"database/sql"
	"errors"
	"log/slog"
	"strings"

	"github.com/google/uuid"

	"github.com/wspl/demi/internal/agent/server"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

type titleRequest struct {
	messages []string
	from     string
	seen     uint64
}

func (s *Shard) askTitle(ctx context.Context, id webapi.ConversationID) error {
	record, err := s.Control().Conversation(ctx, id)
	if err != nil {
		return &TitleRefusal{Kind: TitleStorage, Err: err}
	}
	if record == nil || record.Owner != s.user {
		return &TitleRefusal{Kind: TitleNotFound}
	}
	if record.Archived {
		return &TitleRefusal{Kind: TitleArchived}
	}
	if record.Model == nil {
		return &TitleRefusal{Kind: TitleModelNotSelected}
	}
	provider, err := webapi.ParseProviderID(record.Model.ProviderID)
	if err != nil {
		return &TitleRefusal{Kind: TitleProviderNotFound}
	}
	entry, err := s.services.Vault.Visible(ctx, s.user, provider)
	if err != nil {
		return &TitleRefusal{Kind: TitleStorage, Err: err}
	}
	if entry == nil {
		return &TitleRefusal{Kind: TitleProviderNotFound}
	}
	var blocks []core.Block
	if tree := s.agent.Tree(hostaccess.RootOf(id)); tree != nil {
		blocks = tree.Root().Session().Transcript().Blocks
	} else {
		_, err = s.services.Conversations.Read(ctx, id, func(ctx context.Context, tx *sql.Tx) error {
			history, err := database.ReadHistory(ctx, tx)
			blocks = history.Blocks
			return err
		})
		if err != nil {
			return &TitleRefusal{Kind: TitleStorage, Err: err}
		}
	}
	var messages []string
	for _, block := range blocks {
		var content []core.UserContentBlock
		switch block := block.(type) {
		case *core.UserBlock:
			content = block.Content
		case *core.SteerBlock:
			content = block.Content
		case *core.AbortBlock, *core.AgentMessageBlock, *core.CompactionBoundaryBlock, *core.CompactionMarkerBlock, *core.ContextBlock, *core.ErrorBlock, *core.RedactedThinkingBlock, *core.ResponseBlock, *core.ResumeBlock, *core.TextBlock, *core.ThinkingBlock, *core.ToolCallBlock, *core.WakeupBlock:
			continue
		}
		var texts []string
		for _, part := range content {
			if text, ok := part.(*core.UserText); ok {
				texts = append(texts, text.Text)
			}
		}
		text := strings.Join(texts, "\n")
		if strings.TrimSpace(text) != "" {
			messages = append(messages, text)
		}
	}
	if len(messages) == 0 {
		return &TitleRefusal{Kind: TitleNoMessages}
	}
	s.startTitle(id, *record.Model, titleRequest{messages: messages, from: record.Title, seen: record.UserMessages})
	return nil
}
func (s *Shard) startTitle(id webapi.ConversationID, selection core.ModelSelection, request titleRequest) {
	if !s.services.ConversationTuning.Titles {
		return
	}
	s.mu.Lock()
	if s.closing || s.titles[id] != nil {
		s.mu.Unlock()
		return
	}
	ctx, cancel := context.WithCancel(s.ctx)
	task := &idleWatch{cancel: cancel, done: make(chan struct{})}
	if s.titles == nil {
		s.titles = make(map[webapi.ConversationID]*idleWatch)
	}
	s.titles[id] = task
	s.work.Add(1)
	s.mu.Unlock()
	s.Mark(pagesync.Part{Kind: pagesync.Conversation, ConversationID: id})
	go func() {
		defer s.work.Done()
		defer close(task.done)
		defer cancel()
		if err := s.generateTitle(ctx, id, selection, request); err != nil && ctx.Err() == nil {
			slog.Warn("a title request wrote nothing", "conversation", id, "error", err)
		}
		s.mu.Lock()
		ours := s.titles[id] == task
		if ours {
			delete(s.titles, id)
		}
		s.mu.Unlock()
		if ours {
			s.Mark(pagesync.Part{Kind: pagesync.Conversation, ConversationID: id})
		}
	}()
}
func (s *Shard) generateTitle(ctx context.Context, id webapi.ConversationID, selection core.ModelSelection, request titleRequest) error {
	runtime, err := s.providers.Runtime(ctx, hostaccess.RootOf(id), selection)
	if err != nil {
		return err
	}
	title, err := server.Title(ctx, runtime, string(id), uuid.NewString(), selection, request.messages)
	err = errors.Join(err, runtime.Close(context.WithoutCancel(ctx)))
	if err != nil {
		return err
	}
	if title == nil || ctx.Err() != nil {
		return nil
	}
	_, err = s.Control().GeneratedTitle(ctx, id, *title, request.from, request.seen)
	return err
}
