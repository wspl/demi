package usershard

import (
	"context"
	"database/sql"
	"errors"
	"strings"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/pagesync"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/webapi"
)

func (s *Shard) fork(ctx context.Context, source, destination webapi.ConversationID, block core.BlockID) (Forked, error) {
	permit, err := s.forks.Acquire(ctx, strings.ToLower(string(destination)))
	if err != nil {
		return Forked{}, err
	}
	defer permit.Release()
	ctx = context.WithoutCancel(ctx)
	control := s.services.Control
	record, err := control.Conversation(ctx, source)
	if err != nil {
		return Forked{}, &ForkRefusal{Kind: ForkStorage, Err: err}
	}
	if record == nil || record.Owner != s.user {
		return Forked{}, &ForkRefusal{Kind: ForkSourceNotFound}
	}
	reserved, err := control.ForkOperation(ctx, destination)
	if err != nil {
		return Forked{}, &ForkRefusal{Kind: ForkStorage, Err: err}
	}
	if reserved != nil && !reserved.SameAttempt(s.user, record.ID, block) {
		return Forked{}, &ForkRefusal{Kind: ForkConflict}
	}
	existing, err := control.Conversation(ctx, destination)
	if err != nil {
		return Forked{}, &ForkRefusal{Kind: ForkStorage, Err: err}
	}
	if existing != nil {
		if reserved == nil {
			return Forked{}, &ForkRefusal{Kind: ForkUnavailable}
		}
		return Forked{Record: *existing}, nil
	}
	if reserved != nil {
		committed, err := forkCommitted(ctx, s.services.Conversations, destination)
		if err != nil {
			return Forked{}, &ForkRefusal{Kind: ForkStorage, Err: err}
		}
		if committed {
			published, err := control.PublishFork(ctx, destination)
			if err != nil {
				return Forked{}, &ForkRefusal{Kind: ForkStorage, Err: err}
			}
			s.published(published)
			return Forked{Record: published}, nil
		}
	}
	seed, err := s.agent.PrepareFork(ctx, hostaccess.RootOf(record.ID), block)
	if err != nil {
		return Forked{}, forkRefused(err)
	}
	if reserved == nil {
		target := record.Target
		if cloud, ok := target.(*webapi.ConversationTargetCloud); ok && cloud.Path == nil {
			resolved, err := hostaccess.ResolveTarget(ctx, s, *record)
			if err != nil {
				return Forked{}, &ForkRefusal{Kind: ForkStorage, Err: err}
			}
			path := database.ExecutionPath(resolved)
			target = &webapi.ConversationTargetCloud{Path: &path}
		}
		attached, err := control.AttachedHosts(ctx, record.ID)
		if err != nil {
			return Forked{}, &ForkRefusal{Kind: ForkStorage, Err: err}
		}
		reserved, err = control.ReserveFork(ctx, database.ForkOperation{ID: destination, Owner: s.user, Source: record.ID, Block: block, Metadata: database.ForkMetadata{Title: record.Title + " (Fork)", Target: target, Model: record.Model, CreatedAt: s.Clock().Now(), AttachedHosts: attached}})
		if err != nil {
			return Forked{}, &ForkRefusal{Kind: ForkStorage, Err: err}
		}
		if reserved == nil {
			return Forked{}, &ForkRefusal{Kind: ForkUnavailable}
		}
	}
	commands := database.CommandsOf(seed.Transcript)
	var rows []database.CommandOutput
	var numbers []database.SequenceNext
	_, err = s.services.Conversations.Read(ctx, source, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		rows, err = database.CommandOutputRows(ctx, tx, commands)
		if err != nil {
			return err
		}
		numbers, err = database.Sequences(ctx, tx)
		return err
	})
	if err != nil {
		return Forked{}, &ForkRefusal{Kind: ForkStorage, Err: err}
	}
	err = s.ConversationDB(destination).Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		if len(rows) != 0 {
			if err := database.InsertCommandOutputs(ctx, tx, s.BlobUses(), rows); err != nil {
				return err
			}
		}
		return database.ContinueSequences(ctx, tx, numbers)
	})
	if err != nil {
		return Forked{}, &ForkRefusal{Kind: ForkStorage, Err: err}
	}
	if reserved.Metadata.Model != nil {
		seed.State.Model = *reserved.Metadata.Model
	}
	if err := s.agent.InitializeFork(ctx, hostaccess.RootOf(destination), seed); err != nil {
		return Forked{}, forkRefused(err)
	}
	published, err := control.PublishFork(ctx, destination)
	if err != nil {
		return Forked{}, &ForkRefusal{Kind: ForkStorage, Err: err}
	}
	s.published(published)
	return Forked{Record: published, Created: true}, nil
}

func forkRefused(err error) error {
	kind := ForkTarget
	var failure *session.ForkError
	if errors.As(err, &failure) && failure.Kind == session.ForkStore {
		kind = ForkFailed
	}
	return &ForkRefusal{Kind: kind, Message: err.Error(), Err: err}
}
func (s *Shard) published(record database.ConversationRecord) {
	s.Mark(pagesync.Part{Kind: pagesync.Conversation, ConversationID: record.ID})
	s.Mark(pagesync.Part{Kind: pagesync.ConversationOrder})
}

func forkCommitted(ctx context.Context, stores *database.ConversationStores, id webapi.ConversationID) (bool, error) {
	committed := false
	_, err := stores.Read(ctx, id, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		committed, err = database.HasRoot(ctx, tx)
		return err
	})
	return committed, err
}
