package usershard

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"time"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/webapi"
)

const blobGrace = 24 * time.Hour

func (s *Shard) retentionPass(ctx context.Context) error {
	conversations, err := s.services.Control.ConversationOrder(ctx, s.user)
	if err != nil {
		return err
	}
	var failures []error
	for _, id := range conversations {
		if s.ctx.Err() != nil {
			return errors.Join(failures...)
		}
		failures = append(failures, s.removeOutputs(ctx, id))
		if s.agent.Tree(hostaccess.RootOf(id)) != nil {
			failures = append(failures, s.services.Control.MarkLive(ctx, id, s.Clock().Now()))
		} else {
			failures = append(failures, s.retireMedia(ctx, id, false))
		}
	}
	failures = append(failures, s.collectBlobs(ctx))
	return errors.Join(failures...)
}

func (s *Shard) removeOutputs(ctx context.Context, id webapi.ConversationID) error {
	now := s.Clock().Now()
	at, err := now.Time()
	if err != nil {
		return err
	}
	expired, err := core.TimestampFromTime(at.Add(-time.Duration(store.CommandOutputDays) * 24 * time.Hour))
	if err != nil {
		return err
	}
	due := false
	_, err = s.services.Conversations.Read(ctx, id, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		due, err = database.HasExpiredOutputs(ctx, tx, expired)
		return err
	})
	if err != nil || !due {
		return err
	}
	return s.ConversationDB(id).Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		_, err := database.RemoveExpiredOutputs(ctx, tx, s.BlobUses(), expired, now)
		return err
	})
}

func (s *Shard) retirement(ctx context.Context, id webapi.ConversationID) (transcript.Retirement, error) {
	now := s.Clock().Now()
	live, err := s.services.Control.LiveAt(ctx, id)
	if err != nil {
		return transcript.Retirement{}, err
	}
	if live == nil {
		return transcript.Retirement{}, fmt.Errorf("the conversation is gone")
	}
	at, err := now.Time()
	if err != nil {
		return transcript.Retirement{}, err
	}
	last, err := live.Time()
	if err != nil {
		return transcript.Retirement{}, err
	}
	return transcript.Retirement{Now: now, Idle: at.Sub(last) >= transcript.Kept}, nil
}

func (s *Shard) retireMedia(ctx context.Context, id webapi.ConversationID, wait bool) error {
	retirement, err := s.retirement(ctx, id)
	if err != nil {
		return err
	}
	var due []database.RetiredNode
	_, err = s.services.Conversations.Read(ctx, id, func(ctx context.Context, tx *sql.Tx) error {
		var err error
		due, err = database.Retirable(ctx, tx, retirement)
		return err
	})
	if err != nil || len(due) == 0 {
		return err
	}
	var reservation *gates.Reservation
	gate := s.conversations.Slot(id).FileGate()
	for {
		if s.ctx.Err() != nil || s.agent.Tree(hostaccess.RootOf(id)) != nil {
			return nil
		}
		changed := gate.State().Changed()
		reservation = gate.TryReserve()
		if reservation != nil {
			break
		}
		if !wait {
			return nil
		}
		select {
		case <-changed:
		case <-ctx.Done():
			return ctx.Err()
		case <-s.ctx.Done():
			return nil
		}
	}
	defer reservation.Release()
	if s.agent.Tree(hostaccess.RootOf(id)) != nil {
		return nil
	}
	retirement, err = s.retirement(ctx, id)
	if err != nil {
		return err
	}
	return s.ConversationDB(id).Call(ctx, func(ctx context.Context, tx *sql.Tx) error {
		_, err := database.RetireMedia(ctx, tx, s.BlobUses(), retirement)
		return err
	})
}

func (s *Shard) collectBlobs(ctx context.Context) error {
	blobs := s.Blobs()
	blobs.ForgetUses(blobGrace)
	now, err := s.Clock().Now().Time()
	if err != nil {
		return err
	}
	stored, err := blobs.List(ctx)
	if err != nil {
		return fmt.Errorf("the blob namespace cannot be listed: %w", err)
	}
	var old []core.BlobRef
	for _, blob := range stored {
		at, err := blob.Written.Time()
		if err != nil {
			return err
		}
		if now.Sub(at) > blobGrace {
			old = append(old, blob.Blob)
		}
	}
	if len(old) == 0 {
		return nil
	}
	references, err := s.references(ctx)
	if err != nil {
		return err
	}
	for _, blob := range old {
		if s.ctx.Err() != nil {
			break
		}
		if !references[blob] {
			if _, err := blobs.DeleteUnused(ctx, blob, blobGrace); err != nil {
				slog.WarnContext(ctx, "a blob was not deleted", "blob", blob, "error", err)
			}
		}
	}
	return nil
}

func (s *Shard) references(ctx context.Context) (map[core.BlobRef]bool, error) {
	control := s.services.Control
	uploads, err := control.UploadBlobs(ctx, s.user)
	if err != nil {
		return nil, fmt.Errorf("the upload records cannot be read: %w", err)
	}
	plugins, err := control.PluginBlobs(ctx, s.user)
	if err != nil {
		return nil, fmt.Errorf("the plugins' records cannot be read: %w", err)
	}
	result := make(map[core.BlobRef]bool)
	for _, blob := range uploads {
		result[blob] = true
	}
	for _, blob := range plugins {
		result[blob] = true
	}
	conversations, err := control.ConversationOrder(ctx, s.user)
	if err != nil {
		return nil, fmt.Errorf("the conversations cannot be listed: %w", err)
	}
	forks, err := control.PendingForks(ctx)
	if err != nil {
		return nil, fmt.Errorf("the Forks under way cannot be listed: %w", err)
	}
	for _, fork := range forks {
		if fork.Owner == s.user {
			conversations = append(conversations, fork.ID)
		}
	}
	for _, id := range conversations {
		_, err := s.services.Conversations.Read(ctx, id, func(ctx context.Context, tx *sql.Tx) error {
			refs, err := database.References(ctx, tx)
			for _, blob := range refs {
				result[blob] = true
			}
			return err
		})
		if err != nil {
			return nil, fmt.Errorf("the database of conversation %s cannot be read: %w", id, err)
		}
	}
	return result, nil
}
