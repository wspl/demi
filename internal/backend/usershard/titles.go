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
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

type titleRequest struct {
	messages []string
	from     string
	seen     uint64
}

func (s *Shard) askTitle(ctx context.Context, id webapiproto.ConversationID) error {
	record, found, err := s.Control().Conversation(ctx, id)
	if err != nil {
		return err
	}
	if !found || record.Owner != s.user {
		return ErrConversationNotFound
	}
	if record.Archived {
		return ErrArchivedChange
	}
	if record.Model == nil {
		return ErrModelNotSelected
	}
	provider, err := webapiproto.ParseProviderID(record.Model.ProviderID)
	if err != nil {
		return ErrProviderNotFound
	}
	entry, err := s.services.Vault.Visible(ctx, s.user, provider)
	if err != nil {
		return err
	}
	if entry == nil {
		return ErrProviderNotFound
	}
	var blocks []types.Block
	if tree := s.agent.Tree(hostaccess.RootOf(id)); tree != nil {
		blocks = tree.Root().Session().Transcript().Blocks
	} else {
		_, err = s.services.Conversations.Read(ctx, id, func(ctx context.Context, tx *sql.Tx) error {
			history, err := database.ReadHistory(ctx, tx)
			blocks = history.Blocks
			return err
		})
		if err != nil {
			return err
		}
	}
	messages := titleMessages(blocks)
	if len(messages) == 0 {
		return ErrNoMessages
	}
	s.startTitle(id, *record.Model, titleRequest{
		messages: messages,
		from:     record.Title,
		seen:     record.UserMessages,
	})
	return nil
}

func (s *Shard) startTitle(
	id webapiproto.ConversationID,
	selection types.ModelSelection,
	request titleRequest,
) {
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
		s.titles = make(map[webapiproto.ConversationID]*idleWatch)
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

func (s *Shard) generateTitle(
	ctx context.Context,
	id webapiproto.ConversationID,
	selection types.ModelSelection,
	request titleRequest,
) error {
	runtime, err := s.providers.Runtime(ctx, hostaccess.RootOf(id), selection)
	if err != nil {
		return err
	}
	title, err := server.Title(ctx, runtime, string(id), uuid.NewString(), selection, request.messages)
	closeErr := runtime.Close(context.WithoutCancel(ctx))
	err = errors.Join(err, closeErr)
	if err != nil {
		return err
	}
	if title == "" || ctx.Err() != nil {
		return nil
	}
	_, err = s.Control().GeneratedTitle(ctx, id, title, request.from, request.seen)
	return err
}

// titleMessages preserves the user and steer text used to request a conversation title.
func titleMessages(blocks []types.Block) []string {
	var messages []string
	for _, block := range blocks {
		var content []types.UserContentBlock
		switch block := block.(type) {
		case *types.UserBlock:
			content = block.Content
		case *types.SteerBlock:
			content = block.Content
		case *types.AbortBlock,
			*types.AgentMessageBlock,
			*types.CompactionBoundaryBlock,
			*types.CompactionMarkerBlock,
			*types.ContextBlock,
			*types.ErrorBlock,
			*types.RedactedThinkingBlock,
			*types.ResponseBlock,
			*types.ResumeBlock,
			*types.TextBlock,
			*types.ThinkingBlock,
			*types.ToolCallBlock,
			*types.WakeupBlock:
			continue
		}
		var texts []string
		for _, part := range content {
			if text, ok := part.(*types.UserText); ok {
				texts = append(texts, text.Text)
			}
		}
		text := strings.Join(texts, "\n")
		if strings.TrimSpace(text) != "" {
			messages = append(messages, text)
		}
	}
	return messages
}
