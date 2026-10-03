package usershard

import (
	"context"
	"errors"
	"sync"
	"time"

	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/hostaccess"
	"github.com/wspl/demi/internal/backend/idlewatch"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/webapi"
)

type idleWatch struct {
	cancel context.CancelFunc
	done   chan struct{}
}
type conversationIdle struct {
	shard *Shard
	id    webapi.ConversationID
}

func (p conversationIdle) Check(context.Context) (idlewatch.Activity, error) {
	return p.shard.Activity(p.id), nil
}
func (p conversationIdle) Changed() <-chan struct{} {
	return p.shard.conversations.Slot(p.id).FileGate().State().Changed()
}
func (p conversationIdle) Reserve(context.Context) (idlewatch.Retirement, error) {
	hold := p.shard.HoldForIdle(p.id)
	if hold == nil {
		return nil, nil
	}
	return &idleRetirement{conversationIdle: p, hold: hold}, nil
}

type idleRetirement struct {
	conversationIdle
	hold cloud.ConversationHold
}

func (r *idleRetirement) Release() { r.hold.Release() }
func (r *idleRetirement) Retire(ctx context.Context) error {
	ctx = context.WithoutCancel(ctx)
	record, err := hostaccess.OwnedConversation(ctx, r.shard, r.id)
	if err != nil {
		return err
	}
	return hostaccess.ReleaseEverywhere(ctx, r.shard, record)
}

type conversationHold struct {
	once      sync.Once
	tree      *gates.Reservation
	files     *gates.Reservation
	transfers *hostaccess.TransfersClosed
	reset     bool
}

func (h *conversationHold) Release() {
	h.once.Do(func() {
		if !h.reset && h.tree != nil {
			h.tree.Release()
		}
		if h.files != nil {
			h.files.Release()
		}
		if h.transfers != nil {
			h.transfers.Release()
		}
		if h.reset && h.tree != nil {
			h.tree.Release()
		}
	})
}

func (s *Shard) holdReset(ctx context.Context, id webapi.ConversationID, filesOnCloud bool, wait time.Duration) (cloud.ConversationHold, error) {
	hold := &conversationHold{reset: true}
	succeeded := false
	defer func() {
		if !succeeded {
			hold.Release()
		}
	}()
	if tree := s.agent.Tree(hostaccess.RootOf(id)); tree != nil {
		timed, cancel := context.WithTimeout(ctx, wait)
		reserved, err := tree.Interrupt(timed)
		cancel()
		if err != nil {
			if ctx.Err() != nil {
				return nil, ctx.Err()
			}
			if errors.Is(err, context.DeadlineExceeded) {
				return nil, nil
			}
			return nil, err
		}
		hold.tree = reserved
	}
	slot := s.conversations.Slot(id)
	if filesOnCloud {
		ended, err := slot.Transfers().Close(ctx)
		if err != nil {
			return nil, err
		}
		hold.transfers = ended
	}
	timed, cancel := context.WithTimeout(ctx, wait)
	defer cancel()
	files, err := slot.FileGate().Reserve(timed)
	if err != nil {
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		if errors.Is(err, context.DeadlineExceeded) {
			return nil, nil
		}
		return nil, err
	}
	hold.files = files
	succeeded = true
	return hold, nil
}

// stopIdle invalidates the old watch before a target change starts a new window.
// Retirement already admitted owns its reservation and finishes independently.
func (s *Shard) stopIdle(id webapi.ConversationID) {
	s.mu.Lock()
	watch := s.idle[id]
	delete(s.idle, id)
	s.mu.Unlock()
	if watch != nil {
		watch.cancel()
	}
}
