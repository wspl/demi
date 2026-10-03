package server

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"slices"
	"strconv"
	"strings"
	"unicode/utf8"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
)

const (
	maxLiveChildren = 8
	resultMaxBytes  = 32 * 1024
)

type child[H host.Host] struct {
	node      *Node[H]
	closing   bool // Protected with the tree directory by server.mu.
	failure   *string
	done      chan struct{}
	events    *session.Subscription
	telemetry telemetry
	delivery  gates.Serial // Orders message admission with natural closing.
}

func (t *Tree[H]) childrenOf(owner core.NodeID) []*child[H] {
	t.server.mu.Lock()
	result := []*child[H]{}
	for _, c := range t.children {
		if *c.node.record.Parent == owner {
			result = append(result, c)
		}
	}
	t.server.mu.Unlock()
	slices.SortFunc(result, func(a, b *child[H]) int {
		if a.node.record.Number < b.node.record.Number {
			return -1
		}
		if a.node.record.Number > b.node.record.Number {
			return 1
		}
		return 0
	})
	return result
}

func (t *Tree[H]) descendants(owner core.NodeID) []*child[H] {
	result := []*child[H]{}
	for _, child := range t.childrenOf(owner) {
		result = append(result, child)
		result = append(result, t.descendants(child.node.ID())...)
	}
	return result
}

func (t *Tree[H]) isChildOf(id, owner core.NodeID) bool {
	t.server.mu.Lock()
	defer t.server.mu.Unlock()
	c := t.children[id]
	return c != nil && *c.node.record.Parent == owner
}

// ownerLocked checks a child start against the same directory decision as closing.
func (t *Tree[H]) ownerLocked(id core.NodeID) (*Node[H], error) {
	if t.disposing {
		return nil, errors.New("owner session is closing")
	}
	if id == t.id {
		return t.root, nil
	}
	c := t.children[id]
	if c == nil {
		return nil, errors.New("this session is not in the agent directory")
	}
	if c.closing {
		return nil, errors.New("owner session is closing")
	}
	return c.node, nil
}

func (t *Tree[H]) profile(name *string) (*core.Profile, error) {
	if name == nil {
		return nil, nil
	}
	names := []string{}
	for i := range t.profiles {
		profile := &t.profiles[i]
		if profile.Name == *name {
			return profile, nil
		}
		names = append(names, profile.Name)
	}
	available := strings.Join(names, ", ")
	if available == "" {
		available = "none; omit --profile to inherit the parent"
	}
	return nil, fmt.Errorf("unknown profile %q (available: %s)", *name, available)
}

func (t *Tree[H]) checkCapacity(owner core.NodeID) error {
	if len(t.childrenOf(owner)) >= maxLiveChildren {
		return errors.New("at most 8 running subagents per session; abort one or wait for a result")
	}
	return nil
}

func (t *Tree[H]) startChild(
	ctx context.Context,
	owner *Node[H],
	record store.NodeRecord,
	first *core.QueuedMessage,
) error {
	profile, err := t.profile(record.Profile)
	if err != nil {
		return err
	}
	lease, err := t.admission.Enter(ctx, gates.Demand)
	if err != nil {
		return err
	}
	defer lease.Release()
	runtime, err := owner.session.ForkRuntime(ctx)
	if err != nil {
		return err
	}
	model, instructions, inherited := owner.session.Model(), owner.runtime.instructions, owner.runtime.inherited
	model, instructions, inherited = childProfile(profile, model, instructions, inherited)
	preamble := subagentPreamble(record.Number, owner.record.Number, record.CanSpawnSubagents)
	node, continuation, err := t.assemble(ctx, assembly{
		record:       record,
		cwd:          owner.CWD(),
		model:        model,
		runtime:      runtime,
		instructions: instructions,
		preamble:     &preamble,
		inherited:    inherited,
		first:        first,
	})
	if err != nil {
		return err
	}
	now, err := t.server.deps.Clock.Now().Millisecond()
	if err != nil {
		return errors.Join(err, node.session.Dispose(context.WithoutCancel(ctx)))
	}
	c := &child[H]{node: node, done: make(chan struct{}), telemetry: telemetry{lastEvent: now}}
	t.observeChild(c, node)
	t.publishChild(c, node)
	if continuation != nil {
		if err := node.continueFrom(ctx, *continuation); err != nil {
			t.report(fmt.Errorf("subagent %s did not save its start: %w", node.ID(), err))
		}
	}
	lease.Release()
	t.restoreChildren(ctx, node)
	t.server.mu.Lock()
	if !t.disposing {
		t.workers.Go(func() { t.supervise(c) })
	}
	t.server.mu.Unlock()
	return nil
}

func (t *Tree[H]) restoreChildren(ctx context.Context, owner *Node[H]) {
	permit, err := t.starts.Acquire(ctx, owner.ID())
	if err != nil {
		t.report(err)
		return
	}
	defer permit.Release()
	records, err := t.store.Children(ctx, owner.ID())
	if err != nil {
		t.report(fmt.Errorf("the children of %s were not restored: %w", owner.ID(), err))
		return
	}
	for _, record := range records {
		t.server.mu.Lock()
		disposing, live := t.disposing, t.children[record.ID] != nil
		t.server.mu.Unlock()
		if disposing {
			return
		}
		if live {
			continue
		}
		if record.Closed != nil {
			if !record.Delivered {
				t.deliver(ctx, owner, record, *record.Closed)
			}
			continue
		}
		if err := t.startChild(ctx, owner, record, nil); err != nil {
			slog.Warn("a subagent could not be restored; deleting its subtree", "node", record.ID, "error", err)
			if err := t.store.DeleteNode(ctx, record.ID); err != nil {
				t.report(fmt.Errorf("subagent %s was not deleted: %w", record.ID, err))
			}
		}
	}
}

func (t *Tree[H]) supervise(c *child[H]) {
	for {
		changed := t.changes()
		status := c.node.session.Status()
		permit, err := c.delivery.Acquire(t.ctx)
		if err != nil {
			return
		}
		status = c.node.session.Status()
		t.server.mu.Lock()
		descendants := 0
		for _, child := range t.children {
			if *child.node.record.Parent == c.node.ID() {
				descendants++
			}
		}
		stop := t.disposing || c.closing
		var phase store.ClosePhase
		if !stop {
			if c.failure != nil {
				phase = &store.Failed{Failure: *c.failure}
			} else if status.Settle == session.Settled && !status.Wakeups && !status.AgentInput &&
				descendants == 0 && t.changing[c.node.ID()] == 0 {
				phase = &store.Completed{}
			}
			if phase != nil {
				c.closing = true
				t.lifecycle.Add(1)
			}
		}
		t.server.mu.Unlock()
		permit.Release()
		if stop {
			return
		}
		if phase != nil {
			go func() {
				defer t.lifecycle.Done()
				t.closeChild(context.Background(), c, phase)
			}()
			return
		}
		select {
		case <-t.ctx.Done():
			return
		case <-changed:
		case <-status.Changed():
		}
	}
}

func (t *Tree[H]) abortChild(ctx context.Context, id core.NodeID) error {
	t.server.mu.Lock()
	c := t.children[id]
	if c == nil || t.disposing {
		t.server.mu.Unlock()
		return nil
	}
	start := !c.closing
	if start {
		c.closing = true
		t.lifecycle.Add(1)
	}
	t.server.mu.Unlock()
	if start {
		go func() {
			defer t.lifecycle.Done()
			t.closeChild(context.Background(), c, &store.Aborted{})
		}()
	}
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-c.done:
		return nil
	}
}

func (t *Tree[H]) abortChildren(ctx context.Context, owner core.NodeID) error {
	var errs []error
	for _, c := range t.childrenOf(owner) {
		errs = append(errs, t.abortChild(ctx, c.node.ID()))
	}
	return errors.Join(errs...)
}

func (t *Tree[H]) closeChild(ctx context.Context, c *child[H], phase store.ClosePhase) {
	defer close(c.done)
	record := c.node.record
	t.server.mu.Lock()
	t.changing[*record.Parent]++
	t.server.mu.Unlock()
	defer t.endChange(*record.Parent)
	owner := t.Node(*record.Parent)
	if owner != nil {
		lease, err := owner.runtime.lifecycle.Enter(ctx, gates.Maintenance)
		if err != nil {
			t.report(err)
			return
		}
		defer lease.Release()
	}
	agent := c.node.session
	if _, completed := phase.(*store.Completed); !completed {
		if !t.stopChild(ctx, agent, record.ID) {
			return
		}
	} else {
		phase = &store.Completed{Result: boundedResult(agent.LastAssistantText())}
	}
	if err := agent.Dispose(ctx); err != nil {
		t.report(fmt.Errorf("subagent %s did not save its final checkpoint: %w", record.ID, err))
	}
	c.events.Release()
	ended := store.NodeClose{Phase: phase, At: t.server.deps.Clock.Now()}
	err := t.store.CloseNode(ctx, record.ID, ended)
	t.server.mu.Lock()
	delete(t.children, record.ID)
	t.server.mu.Unlock()
	t.bump()
	t.frames.Lock()
	t.releaseChildObservations(record.ID)
	if err == nil {
		record.Closed = &ended
		t.publish(&framewire.SubagentFrame{Event: framewire.SubagentEventClosed, Job: *record.Job()})
	}
	t.frames.Unlock()
	if err != nil {
		t.report(fmt.Errorf("subagent %s did not close: %w", record.ID, err))
		return
	}
	if owner != nil {
		t.deliver(ctx, owner, record, ended)
	}
}

func (t *Tree[H]) deliver(ctx context.Context, owner *Node[H], record store.NodeRecord, ended store.NodeClose) {
	id, err := (core.CompletionID{Child: record.ID, Round: record.Round}).BlockID()
	if err != nil {
		t.report(err)
		return
	}
	content, outcome := "", core.CompletionOutcome("aborted")
	switch phase := ended.Phase.(type) {
	case *store.Completed:
		content, outcome = phase.Result, "completed"
	case *store.Failed:
		content, outcome = phase.Failure, "failed"
	case *store.Aborted:
	}
	message := core.AgentMessage{
		ID: id,
		Sender: core.Sender{
			ID:          record.ID,
			Number:      record.Number,
			Description: record.Description,
			Round:       record.Round,
		},
		RecipientID: owner.ID(),
		Timestamp:   ended.At,
		Content:     content,
		Event:       &core.CompletionEvent{Outcome: outcome},
	}
	if err := owner.session.AcceptAgentMessage(ctx, message); err != nil {
		var refusal *session.AgentMessageError
		if !errors.As(err, &refusal) || refusal.Kind != session.AgentMessageClosed {
			t.report(fmt.Errorf("the completion of subagent %s was not delivered: %w", record.ID, err))
		}
	}
}

func (t *Tree[H]) sendMessage(ctx context.Context, caller core.NodeID, target, content string) (uint64, error) {
	sender := t.Node(caller)
	if sender == nil {
		return 0, errors.New("this session is not in the agent directory")
	}
	noLive := fmt.Errorf(
		"no live agent %q (see `demi agent list`; an archived child is revived only by its parent via resume)",
		target,
	)
	recipient, err := t.recipient(sender, target)
	if err != nil {
		return 0, err
	}
	if recipient == nil {
		return 0, noLive
	}
	if recipient.ID() == caller {
		return 0, errors.New("cannot message your own session")
	}
	if recipient.ID() != t.id {
		permit, err := t.messagePermit(ctx, recipient, noLive)
		if err != nil {
			return 0, err
		}
		defer permit.Release()
	}
	description := sender.record.Description
	if sender.record.Parent == nil {
		description = "root session"
	}
	id, err := core.ParseBlockID(t.server.deps.IDs.NextID())
	if err != nil {
		return 0, err
	}
	message := core.AgentMessage{
		ID: id,
		Sender: core.Sender{
			ID:          caller,
			Number:      sender.record.Number,
			Description: description,
			Round:       sender.record.Round,
		},
		RecipientID: recipient.ID(),
		Timestamp:   t.server.deps.Clock.Now(),
		Content:     content,
		Event:       &core.MessageEvent{},
	}
	if err := recipient.session.AcceptAgentMessage(ctx, message); err != nil {
		return 0, err
	}
	return recipient.record.Number, nil
}

func (t *Tree[H]) childFrames(connection *Connection[H], c *child[H]) []framewire.ServerFrame {
	snapshot := t.observeConnection(connection, c.node).Transcript
	return []framewire.ServerFrame{
		&framewire.SubagentFrame{Event: framewire.SubagentEventStarted, Job: *c.node.record.Job()},
		&framewire.SubagentTranscriptResetFrame{
			SubagentID: c.node.ID(),
			Blocks:     snapshot.Blocks,
			Revision:   snapshot.Version.Revision,
		},
	}
}

func (t *Tree[H]) replay(connection *Connection[H]) []framewire.ServerFrame {
	frames := []framewire.ServerFrame{}
	for _, c := range t.descendants(t.id) {
		frames = append(frames, t.childFrames(connection, c)...)
	}
	return frames
}

// boundedResult cuts a child's completion at its UTF-8 byte bound.
func boundedResult(text string) string {
	if len(text) <= resultMaxBytes {
		return text
	}
	end := resultMaxBytes
	for !utf8.RuneStart(text[end]) {
		end--
	}
	return text[:end]
}

func subagentPreamble(child, parent uint64, spawning bool) string {
	ability := "This session may not spawn subagents."
	if spawning {
		ability = "`demi agent spawn` spawns your own children."
	}
	return strings.Join([]string{
		fmt.Sprintf(
			"You are a subagent: agent %d of this conversation, spawned by agent %d. "+
				"Your transcript starts empty; the task brief in the first user message is your entire "+
				"context.",
			child,
			parent,
		),
		"When you end your turn with nothing pending — no queued messages, no scheduled wakeups, " +
			"no running children of your own — the session ends and your last assistant text is " +
			"returned to the parent as the result. " +
			"Write it for the parent agent, in the shape the task brief asked for.",
		ability,
		"`demi agent send <id|parent>` delivers useful interim information, questions, or " +
			"blockers through internal steering or an idle wakeup. " +
			"It reads the message only from stdin (use a quoted heredoc). " +
			"Your final answer is delivered automatically; do not send a duplicate final result. " +
			"`demi agent list` renders the whole agent tree with your position.",
		"You are not talking to the product user; do not address them.",
	}, "\n")
}

func childProfile(
	profile *core.Profile,
	model core.ModelSelection,
	instructions string,
	inherited *host.CommandSet,
) (core.ModelSelection, string, *host.CommandSet) {
	if profile == nil {
		return model, instructions, inherited
	}
	if profile.Model != nil {
		model = *profile.Model
	}
	if profile.Instructions != nil {
		instructions = *profile.Instructions
	}
	if profile.Commands != nil {
		inherited = inherited.Filter(func(path []string) bool {
			for _, keep := range *profile.Commands {
				if len(path) >= len(keep) && slices.Equal(path[:len(keep)], keep) {
					return true
				}
			}
			return false
		})
	}
	return model, instructions, inherited
}

// stopChild stops the child subtree; false means start admission failed.
func (t *Tree[H]) stopChild(ctx context.Context, agent *session.Session, id core.NodeID) bool {
	agent.Hold()
	for {
		result, err := agent.Abort(ctx)
		if err != nil {
			t.report(err)
			break
		}
		if result.Target == nil {
			break
		}
	}
	permit, err := t.starts.Acquire(ctx, id)
	if err != nil {
		t.report(err)
		return false
	}
	if err := t.abortChildren(ctx, id); err != nil {
		t.report(err)
	}
	permit.Release()
	return true
}

func (t *Tree[H]) recipient(sender *Node[H], target string) (*Node[H], error) {
	if target == "parent" {
		if sender.record.Parent == nil {
			return nil, errors.New("the root session has no parent")
		}
		return t.Node(*sender.record.Parent), nil
	}
	number, err := strconv.ParseUint(target, 10, 64)
	if err != nil {
		return nil, nil
	}
	if number == 0 {
		return t.root, nil
	}
	for _, c := range t.descendants(t.id) {
		if c.node.record.Number == number {
			return c.node, nil
		}
	}
	return nil, nil
}

func (t *Tree[H]) observeChild(c *child[H], node *Node[H]) {
	_, c.events = node.session.Observe(func(event session.Event) {
		switch e := event.(type) {
		case *session.TranscriptChanged:
			t.observe(c, e)
		case *session.ActionFailed:
			t.server.mu.Lock()
			c.failure = new(e.Report.Message)
			t.server.mu.Unlock()
			t.bump()
		case *session.EditCommitted,
			*session.PhaseChanged,
			*session.QueueChanged,
			*session.PendingSteersChanged,
			*session.RetryScheduled,
			*session.ErrorEvent:
		}
	})
}

// releaseChildObservations releases child cursors while the tree frame lock is held.
func (t *Tree[H]) releaseChildObservations(id core.NodeID) {
	for _, connection := range t.connections() {
		t.server.mu.Lock()
		subscription := connection.observations[id]
		delete(connection.observations, id)
		t.server.mu.Unlock()
		if subscription != nil {
			subscription.Release()
		}
	}
}

// messagePermit orders message admission with child closing.
func (t *Tree[H]) messagePermit(ctx context.Context, recipient *Node[H], noLive error) (*gates.Permit, error) {
	t.server.mu.Lock()
	child := t.children[recipient.ID()]
	t.server.mu.Unlock()
	if child == nil {
		return nil, noLive
	}
	permit, err := child.delivery.Acquire(ctx)
	if err != nil {
		return nil, err
	}
	t.server.mu.Lock()
	closing := child.closing
	t.server.mu.Unlock()
	if closing {
		permit.Release()
		return nil, noLive
	}
	return permit, nil
}

func (t *Tree[H]) publishChild(c *child[H], node *Node[H]) {
	t.frames.Lock()
	t.server.mu.Lock()
	t.children[node.ID()] = c
	t.server.mu.Unlock()
	t.bump()
	for _, connection := range t.connections() {
		for _, frame := range t.childFrames(connection, c) {
			connection.send(frame)
		}
	}
	t.frames.Unlock()
}
