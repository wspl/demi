package server

import (
	"context"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/types"
)

// connections snapshots the sockets attached to this conversation.
func (t *Tree[H]) connections() []*Connection[H] {
	t.server.mu.Lock()
	defer t.server.mu.Unlock()
	connections := make([]*Connection[H], 0, len(t.attachments))
	for c := range t.attachments {
		connections = append(connections, c)
	}
	return connections
}

// observeConnection replaces a socket's node observation while frames is held.
// The caller queues the snapshot before releasing frames; callbacks can start
// concurrently with Observe but cannot publish before that snapshot. The initial
// root observer keeps delivering non-transcript events across transcript-only
// syncs, so replacing the transcript cursor cannot discard pending queue/phase
// changes or edit acceptance. Obsolete transcript observers are released.
func (t *Tree[H]) observeConnection(c *Connection[H], node *Node[H]) session.Snapshot {
	var subscription *session.Subscription
	snapshot, subscription := node.session.Observe(func(event session.Event) {
		t.frames.Lock()
		defer t.frames.Unlock()
		t.connectionEventLocked(c, node, subscription, event)
	})
	if node.ID() == t.id {
		c.publishedRevision = snapshot.Transcript.Version.Revision
	}
	t.server.mu.Lock()
	previous := c.observations[node.ID()]
	_, attached := t.attachments[c]
	if attached && node.ID() == t.id && c.stateObservation == nil {
		c.stateObservation = subscription
	}
	state := c.stateObservation
	if attached {
		if c.observations == nil {
			c.observations = make(map[types.NodeID]*session.Subscription)
		}
		c.observations[node.ID()] = subscription
	}
	t.server.mu.Unlock()
	if previous != nil && previous != state {
		previous.Release()
	}
	if !attached {
		subscription.Release()
	}
	return snapshot
}

// waitPublished orders command replies after the transcript changes already
// committed when the command returned. Only the caller waits; callbacks never do.
func (c *Connection[H]) waitPublished(ctx context.Context, tree *Tree[H], revision uint64) error {
	for {
		c.outbox.mu.Lock()
		changed := c.outbox.changed
		ended := c.outbox.state != 0
		c.outbox.mu.Unlock()
		tree.frames.Lock()
		published := c.publishedRevision >= revision
		tree.frames.Unlock()
		if published || ended || c.detached.Load() {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-changed:
		}
	}
}

// connectionEventLocked delivers an event while the tree frame lock is held.
func (t *Tree[H]) connectionEventLocked(
	c *Connection[H],
	node *Node[H],
	subscription *session.Subscription,
	event session.Event,
) {
	t.server.mu.Lock()
	current := c.observations[node.ID()] == subscription
	state := c.stateObservation == subscription
	t.server.mu.Unlock()
	if _, transcript := event.(*session.TranscriptChanged); transcript {
		if !current {
			return
		}
	} else if !state {
		return
	}
	if node.ID() != t.id {
		if e, ok := event.(*session.TranscriptChanged); ok {
			c.send(
				&conversationproto.SubagentTranscriptPatchFrame{
					SubagentID: node.ID(),
					Patches:    e.Patches,
					Revision:   e.Revision,
				},
			)
		}
		return
	}
	if e, ok := event.(*session.EditCommitted); ok {
		reply := c.editReply
		if reply != nil && reply.operation == e.Receipt.OperationID && reply.digest == e.Receipt.Digest {
			c.editReply = nil
			c.send(
				&conversationproto.EditResultFrame{
					OperationID: e.Receipt.OperationID,
					Outcome:     &conversationproto.AcceptedEdit{TurnID: e.Receipt.TurnID},
				},
			)
		}
		return
	}
	if e, ok := event.(*session.TranscriptChanged); ok {
		c.publishedRevision = e.Revision
	}
	if frame := frameOf(event); frame != nil {
		c.send(frame)
	}
}
