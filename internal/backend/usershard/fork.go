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

func (s *Shard) fork(
	ctx context.Context,
	source, destination webapi.ConversationID,
	block core.BlockID,
) (Forked, error) {
	key := strings.ToLower(string(destination))
	permit, err := s.forks.Acquire(ctx, key)
	if err != nil {
		return Forked{}, err
	}
	defer permit.Release()
	ctx = context.WithoutCancel(ctx)
	control := s.services.Control
	record, err := s.forkSource(ctx, source)
	if err != nil {
		return Forked{}, err
	}
	reserved, existing, err := s.forkDestination(ctx, destination, record.ID, block)
	if err != nil {
		return Forked{}, err
	}
	if existing != nil {
		return Forked{Record: *existing}, nil
	}
	if reserved != nil {
		published, committed, err := s.resumeCommittedFork(ctx, destination)
		if err != nil {
			return Forked{}, err
		}
		if committed {
			return Forked{Record: published}, nil
		}
	}
	seed, err := s.agent.PrepareFork(ctx, hostaccess.RootOf(record.ID), block)
	if err != nil {
		return Forked{}, forkRefused(err)
	}
	if reserved == nil {
		reserved, err = s.reserveConversationFork(ctx, *record, destination, block)
		if err != nil {
			return Forked{}, err
		}
	}
	if err := s.copyForkCommands(ctx, source, destination, seed.Transcript); err != nil {
		return Forked{}, err
	}
	if reserved.Metadata.Model != nil {
		seed.State.Model = *reserved.Metadata.Model
	}
	if err := s.agent.InitializeFork(ctx, hostaccess.RootOf(destination), seed); err != nil {
		return Forked{}, forkRefused(err)
	}
	published, err := control.PublishFork(ctx, destination)
	if err != nil {
		return Forked{}, err
	}
	s.published(published)
	return Forked{Record: published, Created: true}, nil
}

func forkRefused(err error) error {
	var failure *session.ForkError
	if errors.As(err, &failure) && failure.Kind == session.ForkStore {
		return err
	}
	return &ForkTargetError{Err: err}
}

func (s *Shard) published(record database.ConversationRecord) {
	s.Mark(pagesync.Part{Kind: pagesync.Conversation, ConversationID: record.ID})
	s.Mark(pagesync.Part{Kind: pagesync.ConversationOrder})
}

func forkCommitted(
	ctx context.Context,
	stores *database.ConversationStores,
	id webapi.ConversationID,
) (bool, error) {
	committed := false
	_, err := stores.Read(ctx, id, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		committed, err = database.HasRoot(ctx, tx)
		return err
	})
	return committed, err
}

// reserveConversationFork records the source metadata and resolves an implicit Cloud path.
func (s *Shard) reserveConversationFork(
	ctx context.Context,
	record database.ConversationRecord,
	destination webapi.ConversationID,
	block core.BlockID,
) (*database.ForkOperation, error) {
	target := record.Target
	if cloud, ok := target.(*webapi.ConversationTargetCloud); ok && cloud.Path == nil {
		resolved, err := hostaccess.ResolveTarget(ctx, s, record)
		if err != nil {
			return nil, err
		}
		path := database.ExecutionPath(resolved)
		target = &webapi.ConversationTargetCloud{Path: &path}
	}
	attached, err := s.services.Control.AttachedHosts(ctx, record.ID)
	if err != nil {
		return nil, err
	}
	reserved, err := s.services.Control.ReserveFork(
		ctx,
		database.ForkOperation{
			ID:     destination,
			Owner:  s.user,
			Source: record.ID,
			Block:  block,
			Metadata: database.ForkMetadata{
				Title:         record.Title + " (Fork)",
				Target:        target,
				Model:         record.Model,
				CreatedAt:     s.Clock().Now(),
				AttachedHosts: attached,
			},
		},
	)
	if err != nil {
		return nil, err
	}
	if reserved == nil {
		return nil, ErrIDUnavailable
	}
	return reserved, nil
}

// copyForkCommands copies command outputs and continues the source’s sequence counters.
func (s *Shard) copyForkCommands(
	ctx context.Context,
	source, destination webapi.ConversationID,
	transcript []core.Block,
) error {
	commands := database.CommandsOf(transcript)
	var rows []database.CommandOutput
	var numbers []database.SequenceNext
	_, err := s.services.Conversations.Read(ctx, source, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		rows, err = database.CommandOutputRows(ctx, tx, commands)
		if err != nil {
			return err
		}
		numbers, err = database.Sequences(ctx, tx)
		return err
	})
	if err != nil {
		return err
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
		return err
	}
	return nil
}

// resumeCommittedFork publishes a previously initialized destination without copying it again.
func (s *Shard) resumeCommittedFork(
	ctx context.Context,
	destination webapi.ConversationID,
) (database.ConversationRecord, bool, error) {
	committed, err := forkCommitted(ctx, s.services.Conversations, destination)
	if err != nil {
		return database.ConversationRecord{}, false, err
	}
	if committed {
		published, err := s.services.Control.PublishFork(ctx, destination)
		if err != nil {
			return database.ConversationRecord{}, false, err
		}
		s.published(published)
		return published, true, nil
	}
	return database.ConversationRecord{}, false, nil
}

// forkSource reads the source and refuses conversations the user does not own.
func (s *Shard) forkSource(
	ctx context.Context,
	source webapi.ConversationID,
) (*database.ConversationRecord, error) {
	record, err := s.services.Control.Conversation(ctx, source)
	if err != nil {
		return nil, err
	}
	if record == nil || record.Owner != s.user {
		return nil, ErrConversationNotFound
	}
	return record, nil
}

// forkDestination checks that the destination belongs to this exact fork attempt.
func (s *Shard) forkDestination(
	ctx context.Context,
	destination, source webapi.ConversationID,
	block core.BlockID,
) (*database.ForkOperation, *database.ConversationRecord, error) {
	reserved, err := s.services.Control.ForkOperation(ctx, destination)
	if err != nil {
		return nil, nil, err
	}
	if reserved != nil && !reserved.SameAttempt(s.user, source, block) {
		return nil, nil, ErrForkConflict
	}
	existing, err := s.services.Control.Conversation(ctx, destination)
	if err != nil {
		return nil, nil, err
	}
	if existing != nil {
		if reserved == nil {
			return nil, nil, ErrIDUnavailable
		}
		return reserved, existing, nil
	}
	return reserved, existing, nil
}
