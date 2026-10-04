package session

import (
	"context"
	"reflect"
	"slices"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

// preparing is an internal turn stage; supervisors observe it as streaming.
const preparing Execution = "preparing"

type actionKind uint8

const (
	sendAction actionKind = iota
	retryAction
	resumeAction
	compactAction
	continueAction
	editAction
)

type action struct {
	kind          actionKind
	turn          types.TurnID
	content       []types.UserContentBlock
	answer        *ActionAnswer
	ctx           context.Context
	cancel        context.CancelFunc
	stopped       bool
	shutdown      bool
	ack           chan struct{}
	again         bool
	startRevision uint64
}
type pendingInput struct {
	steer  *types.PendingSteer
	agent  *store.PendingAgentInput
	wakeup *store.ScheduledWakeup
}
type editFlight struct {
	submission EditSubmission
	acceptance *acceptance
	accepted   bool
}

// coreState belongs to Session.mu. No callback, IO, gate acquisition, channel
// operation or join may occur while this state is held. Effects run after unlock.
type coreState struct {
	id               types.NodeID
	cwd              string
	model            types.ModelSelection
	log              *transcript.Log
	commands         *store.CommandStateHistory
	media            store.HeldMedia
	provider         provider.Runtime
	providerBusy     bool
	change           *ModelSwitch
	waitingChange    *ModelSwitch
	retired          []provider.Runtime
	queue            []*action
	active           *action
	stage            Execution
	inputs           []pendingInput
	arrivals         uint64
	wakeups          []store.ScheduledWakeup
	edits            []store.EditReceipt
	edit             *editFlight
	held             bool
	disposing        bool
	interrupted      bool
	generation       uint64
	generationCtx    context.Context
	generationCancel context.CancelFunc
	dirty            bool
	rows             transcript.DirtyRows
	changed          chan struct{}
	listeners        map[uint64]func(Event)
	nextListener     uint64
	effects          []func()
	delivering       bool
	publishedQueue   []types.QueuedMessage
	publishedSteers  []types.PendingSteer
	publishedPhase   types.SessionPhase
	publishedStatus  Status
}

// mutate performs one atomic session decision and publishes its effects in order.
func (s *Session) mutate(change func(*coreState)) {
	s.mu.Lock()
	c := &s.core
	change(c)
	s.commitLocked()
	queue := c.queuedMessagesLocked()
	if !reflect.DeepEqual(queue, c.publishedQueue) {
		c.publishedQueue = queue
		c.dirty = true
		s.eventLocked(&QueueChanged{Queue: queue})
	}
	steers := c.pendingSteersLocked()
	if !reflect.DeepEqual(steers, c.publishedSteers) {
		c.publishedSteers = steers
		s.eventLocked(&PendingSteersChanged{PendingSteers: steers})
	}
	phase := c.phaseLocked()
	if phase != c.publishedPhase {
		c.publishedPhase = phase
		s.eventLocked(&PhaseChanged{Phase: phase})
	}
	old := c.changed
	c.changed = make(chan struct{})
	status := c.statusLocked()
	previous := c.publishedStatus
	if status.Settle != previous.Settle || status.Wakeups != previous.Wakeups ||
		status.AgentInput != previous.AgentInput {
		c.publishedStatus = status
		c.publishedStatus.changed = make(chan struct{})
		if previous.changed != nil {
			c.effects = append(c.effects, func() { close(previous.changed) })
		}
	}
	c.effects = append(c.effects, func() { close(old) })
	if c.delivering {
		s.mu.Unlock()
		return
	}
	c.delivering = true
	s.mu.Unlock()
	s.deliver()
}

// deliver drains committed session effects without holding the state mutex.
func (s *Session) deliver() {
	for {
		s.mu.Lock()
		if len(s.core.effects) == 0 {
			s.core.delivering = false
			s.mu.Unlock()
			return
		}
		effect := s.core.effects[0]
		s.core.effects[0] = nil
		s.core.effects = s.core.effects[1:]
		s.mu.Unlock()
		effect()
	}
}

// eventLocked queues session listener delivery after the current state change.
func (s *Session) eventLocked(event Event) {
	// Fix membership at the decision, not at delivery: a later observer's
	// snapshot already contains this change even if a callback is still pending.
	limit := s.core.nextListener
	s.core.effects = append(s.core.effects, func() {
		// Visit subscription identities in registration order. Reentrant changes
		// enqueue behind this event, and a released slot is skipped.
		var next uint64
		for {
			s.mu.Lock()
			if next >= limit {
				s.mu.Unlock()
				return
			}
			listener := s.core.listeners[next]
			next++
			s.mu.Unlock()
			if listener != nil {
				listener(event)
			}
		}
	})
}

func (s *Session) emit(event Event) { s.mutate(func(_ *coreState) { s.eventLocked(event) }) }

// commitLocked captures the command-state boundaries of a transcript mutation.
func (s *Session) commitLocked() {
	c := &s.core
	batch, changed := c.log.TakePatches()
	if !changed {
		return
	}
	c.rows.Merge(batch.Rows)
	c.dirty = true
	for _, id := range batch.Touched {
		block := c.log.Find(id)
		opens := false
		switch b := block.(type) {
		case *types.UserBlock, *types.ContextBlock:
			opens = true
		case *types.WakeupBlock:
			opens = b.Placement == "new_turn"
		case *types.SteerBlock,
			*types.AgentMessageBlock,
			*types.ResumeBlock,
			*types.AbortBlock,
			*types.ThinkingBlock,
			*types.RedactedThinkingBlock,
			*types.TextBlock,
			*types.ResponseBlock,
			*types.ToolCallBlock,
			*types.ErrorBlock,
			*types.CompactionBoundaryBlock,
			*types.CompactionMarkerBlock:
		}
		if opens {
			c.commands.Capture(id, store.BeforeUser, c.commands.Revision())
		}
		c.commands.Capture(id, store.AfterBlock, c.commands.Revision())
	}
	s.eventLocked(&TranscriptChanged{Patches: batch.Patches, Revision: batch.Revision})
}

func (c *coreState) phaseLocked() types.SessionPhase {
	if c.stage == Compacting {
		return "compacting"
	}
	if c.active != nil || c.stage == Finalizing || c.interrupted {
		return "running"
	}
	return "idle"
}

func (c *coreState) queuedMessagesLocked() []types.QueuedMessage {
	queue := []types.QueuedMessage{}
	for _, a := range c.queue {
		if a.kind == sendAction {
			queue = append(queue, types.QueuedMessage{ID: a.turn, Content: a.content})
		}
	}
	return queue
}

func (c *coreState) pendingSteersLocked() []types.PendingSteer {
	steers := []types.PendingSteer{}
	for _, input := range c.inputs {
		if input.steer != nil {
			steers = append(steers, *input.steer)
		}
	}
	return steers
}

func (c *coreState) agentInputsLocked() []store.PendingAgentInput {
	inputs := []store.PendingAgentInput{}
	for _, input := range c.inputs {
		if input.agent != nil {
			inputs = append(inputs, *input.agent)
		}
	}
	return inputs
}

func (c *coreState) savedWakeupsLocked() []store.ScheduledWakeup {
	wakeups := append([]store.ScheduledWakeup{}, c.wakeups...)
	for _, input := range c.inputs {
		if input.wakeup != nil {
			wakeups = append(wakeups, *input.wakeup)
		}
	}
	return wakeups
}

func (c *coreState) statusLocked() Status {
	status := Status{Settle: Settled, Wakeups: len(c.wakeups) > 0}
	if c.active != nil || c.stage == Finalizing || len(c.queue) > 0 {
		status.Settle = Busy
	}
	if c.disposing && c.active == nil && c.stage != Finalizing {
		status.Settle = Closed
	}
	for _, input := range c.inputs {
		status.AgentInput = status.AgentInput || input.agent != nil
		status.Wakeups = status.Wakeups || input.wakeup != nil
	}
	return status
}
func (c *coreState) preparingEditLocked() bool { return c.edit != nil && !c.edit.accepted }
func (c *coreState) admissionLocked() error {
	if c.disposing {
		return ErrClosed
	}
	if c.preparingEditLocked() {
		return ErrEditing
	}
	return nil
}

func (c *coreState) steerableLocked() error {
	if c.preparingEditLocked() {
		return ErrEditing
	}
	if c.stage == Finalizing {
		return ErrSteerFinishing
	}
	if c.active == nil {
		return ErrSteerNotRunning
	}
	if c.active.stopped || c.disposing {
		return ErrSteerStopped
	}
	return nil
}

func (c *coreState) latestSelectionLocked() types.ModelSelection {
	if c.waitingChange != nil {
		return c.waitingChange.Model
	}
	if c.change != nil {
		return c.change.Model
	}
	return c.model
}

func (c *coreState) canAbortLocked() bool {
	return !c.disposing &&
		((c.active != nil && !c.active.stopped && c.stage != Finalizing) || len(c.queue) > 0 || len(c.wakeups) > 0)
}

// startNextLocked reserves the next session action before admission can race it.
func (s *Session) startNextLocked() {
	c := &s.core
	if c.active != nil || c.stage == Finalizing || c.disposing {
		return
	}
	var a *action
	if len(c.queue) > 0 {
		a = c.queue[0]
		c.queue = c.queue[1:]
	} else if !c.held {
		blocks := c.log.Blocks()
		stopped := false
		if len(blocks) > 0 {
			_, stopped = blocks[len(blocks)-1].(*types.AbortBlock)
		}
		for _, input := range c.inputs {
			if input.wakeup != nil || (input.agent != nil && !stopped) {
				a = &action{kind: continueAction, turn: types.TurnID(s.deps.IDs.NextID())}
				break
			}
		}
	}
	if a == nil {
		c.stage = Idle
		return
	}
	a.ctx, a.cancel = context.WithCancel(context.Background())
	a.ack = make(chan struct{})
	a.startRevision = c.log.Version().Revision
	c.active = a
	c.stage = preparing
	c.held = false
}

// releaseMediaLocked retains only media the next request or waiting input needs.
func (c *coreState) releaseMediaLocked() {
	refs := map[types.BlobRef]struct{}{}
	blocks := c.log.Blocks()
	for _, b := range blocks[transcript.ReplayStart(blocks):] {
		for _, r := range store.References(b) {
			refs[r] = struct{}{}
		}
	}
	for _, q := range c.queue {
		for _, r := range store.ContentReferences(q.content) {
			refs[r] = struct{}{}
		}
	}
	if c.active != nil {
		for _, r := range store.ContentReferences(c.active.content) {
			refs[r] = struct{}{}
		}
	}
	for _, steer := range c.pendingSteersLocked() {
		for _, r := range store.ContentReferences(steer.Content) {
			refs[r] = struct{}{}
		}
	}
	c.media.Retain(refs)
}

// replaceSwitchLocked preserves the pending runtime for a same-provider switch.
func (c *coreState) replaceSwitchLocked(slot **ModelSwitch, change ModelSwitch) {
	if old := *slot; old != nil && old.Runtime != nil {
		if change.Runtime == nil && old.Model.ProviderID == change.Model.ProviderID {
			change.Runtime = old.Runtime
		} else {
			c.retired = append(c.retired, old.Runtime)
		}
	}
	*slot = &change
}

func (c *coreState) removeQueuedLocked(id types.TurnID) *action {
	for i, a := range c.queue {
		if a.kind == sendAction && a.turn == id {
			c.queue = slices.Delete(c.queue, i, i+1)
			return a
		}
	}
	return nil
}
