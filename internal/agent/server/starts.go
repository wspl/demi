package server

import (
	"context"
	"errors"
	"fmt"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
)

// start owns the whole start beyond its durable reservation, even if the job leaves.
func (t *Tree[H]) start(
	ctx context.Context,
	caller core.NodeID,
	input startInput,
	request string,
	port host.RPCPort,
) (uint64, error) {
	t.server.mu.Lock()
	if t.disposing {
		t.server.mu.Unlock()
		return 0, errors.New("owner session is closing")
	}
	t.lifecycle.Add(1)
	t.server.mu.Unlock()
	defer t.lifecycle.Done()
	permit, err := t.starts.Acquire(ctx, caller)
	if err != nil {
		return 0, err
	}
	defer permit.Release()
	t.server.mu.Lock()
	owner, err := t.ownerLocked(caller)
	if err == nil {
		t.changing[caller]++
	}
	t.server.mu.Unlock()
	if err != nil {
		return 0, err
	}
	t.bump()
	defer t.endChange(caller)

	lease := owner.runtime.lifecycle.TryEnter(gates.Maintenance)
	if lease == nil {
		//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
		return 0, errors.New(
			"Cannot change children while a transcript edit is being prepared",
		)
	}
	defer lease.Release()
	fresh, err := t.startReceipt(ctx, input)
	if err != nil {
		return 0, err
	}
	receipt, err := reserveStart(ctx, port, request, fresh)
	if err != nil {
		return 0, err
	}
	return t.finishStart(t.ctx, owner, receipt)
}

func (t *Tree[H]) finishStart(ctx context.Context, owner *Node[H], receipt startReceipt) (uint64, error) {
	t.server.mu.Lock()
	disposing := t.disposing
	t.server.mu.Unlock()
	if disposing {
		return 0, errors.New("owner session is closing")
	}
	record, err := t.store.Node(ctx, receipt.NodeID)
	if err != nil {
		return 0, err
	}
	if record != nil {
		done, number, err := t.restoreStart(ctx, owner, receipt, record)
		if done || err != nil {
			return number, err
		}
	}

	switch v := receipt.Input.(type) {
	case *spawnInput:
		if !owner.record.CanSpawnSubagents {
			return 0, errors.New("this session may not spawn subagents")
		}
		if err := t.checkCapacity(owner.ID()); err != nil {
			return 0, err
		}
		profile, err := t.profile(v.ProfileName)
		if err != nil {
			return 0, err
		}
		number, err := t.store.NextNumber(ctx, core.SequenceAgent)
		if err != nil {
			return 0, err
		}
		canSpawn := !v.IsSpawnForbidden && (profile == nil || profile.CanSpawnSubagents)
		record := store.NodeRecord{
			ID:                receipt.NodeID,
			Number:            number,
			Parent:            new(owner.ID()),
			Description:       v.Description,
			Profile:           v.ProfileName,
			Round:             receipt.Round,
			StartedAt:         t.server.deps.Clock.Now(),
			CanSpawnSubagents: canSpawn,
		}
		brief, err := t.textMessage(v.Prompt)
		if err != nil {
			return 0, err
		}
		if err := t.startChild(ctx, owner, record, &brief); err != nil {
			return 0, err
		}
		return number, nil
	case *resumeInput:
		return t.reopen(ctx, owner, v.ID, v.Message, receipt.Round)
	}
	return 0, errors.New("invalid start reservation")
}

func (t *Tree[H]) textMessage(text string) (core.QueuedMessage, error) {
	id, err := core.ParseTurnID(t.server.deps.IDs.NextID())
	if err != nil {
		return core.QueuedMessage{}, err
	}
	return core.QueuedMessage{ID: id, Content: []core.UserContentBlock{&core.UserText{Text: text}}}, nil
}

func (t *Tree[H]) reopen(
	ctx context.Context,
	owner *Node[H],
	id core.NodeID,
	message string,
	round uint64,
) (uint64, error) {
	record, err := t.store.Node(ctx, id)
	if err != nil {
		return 0, err
	}
	if record == nil || record.Parent == nil || *record.Parent != owner.ID() {
		return 0, errors.New("no such subagent of yours (see `demi agent list`)")
	}
	if t.Node(id) != nil {
		return 0, fmt.Errorf("subagent %d is still running; send it a message instead", record.Number)
	}
	if record.Closed == nil {
		return 0, fmt.Errorf("no archived subagent %d (see `demi agent list`)", record.Number)
	}
	if err := t.checkCapacity(owner.ID()); err != nil {
		return 0, err
	}
	if !record.Delivered {
		//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
		return 0, errors.New(
			"The previous completion is not saved by the parent yet; retry resume after receiving it",
		)
	}
	if _, err := t.profile(record.Profile); err != nil {
		return 0, err
	}
	queued, err := t.textMessage(message)
	if err != nil {
		return 0, err
	}
	at := t.server.deps.Clock.Now()
	if err := t.store.ReopenNode(ctx, id, round, at, queued); err != nil {
		return 0, err
	}
	record.Round, record.StartedAt, record.Closed, record.Delivered = round, at, nil, false
	if err := t.startChild(ctx, owner, *record, nil); err != nil {
		return 0, err
	}
	return record.Number, nil
}

// endChange lets an owner settle after a child creation or completion finishes.
func (t *Tree[H]) endChange(owner core.NodeID) {
	t.server.mu.Lock()
	t.changing[owner]--
	if t.changing[owner] == 0 {
		delete(t.changing, owner)
	}
	t.server.mu.Unlock()
	t.bump()
}

func (t *Tree[H]) startReceipt(ctx context.Context, input startInput) (startReceipt, error) {
	var err error
	fresh := startReceipt{Input: input, Round: 1}
	switch v := input.(type) {
	case *spawnInput:
		fresh.NodeID, err = core.ParseNodeID(t.server.deps.IDs.NextID())
		if err != nil {
			return startReceipt{}, err
		}
	case *resumeInput:
		fresh.NodeID = v.ID
		previous, err := t.store.Node(ctx, v.ID)
		if err != nil {
			return startReceipt{}, err
		}
		if previous != nil {
			fresh.Round = previous.Round + 1
		}
	}
	return fresh, nil
}

// restoreStart returns whether an existing reservation already determines the result.
func (t *Tree[H]) restoreStart(
	ctx context.Context,
	owner *Node[H],
	receipt startReceipt,
	record *store.NodeRecord,
) (bool, uint64, error) {
	if record.Parent == nil || *record.Parent != owner.ID() {
		return false, 0, errors.New("request-id references an agent owned by another session")
	}
	_, spawn := receipt.Input.(*spawnInput)
	if spawn || record.Round == receipt.Round {
		if record.Closed == nil && t.Node(record.ID) == nil {
			if err := t.startChild(ctx, owner, *record, nil); err != nil {
				return false, 0, err
			}
		}
		return true, record.Number, nil
	}
	if record.Round > receipt.Round {
		return false, 0, errors.New("resume request has been superseded by a later round")
	}
	return false, 0, nil
}
