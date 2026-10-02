package session

import (
	"context"
	"reflect"
	"slices"
	"sync"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/provider"
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
	turn          core.TurnID
	content       []core.UserContentBlock
	answer        *ActionHandle
	ctx           context.Context
	cancel        context.CancelFunc
	stopped       bool
	shutdown      bool
	ack           chan struct{}
	again         bool
	startRevision uint64
}
type pendingInput struct {
	steer  *core.PendingSteer
	agent  *store.PendingAgentInput
	wakeup *store.ScheduledWakeup
}
type editFlight struct {
	submission EditSubmission
	acceptance *Acceptance
	accepted   bool
}

// coreState belongs to Session.mu. No callback, IO, gate acquisition, channel
// operation or join may occur while this state is held. Effects run after unlock.
type coreState struct {
	id               core.NodeID
	cwd              string
	model            core.ModelSelection
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
	publishedQueue   []core.QueuedMessage
	publishedSteers  []core.PendingSteer
	publishedPhase   core.SessionPhase
	publishedStatus  Status
}

// sessionOwner owns all workers and save ordering independently of request waits.
type sessionOwner struct {
	mu       sync.Mutex
	core     coreState
	deps     Deps
	persist  gates.Serial
	ctx      context.Context
	cancel   context.CancelFunc
	workers  sync.WaitGroup
	closed   chan struct{}
	closeErr error
}

// mutate performs one atomic session decision and publishes its effects in order.
func (s *Session) mutate(change func(*coreState)) {
	s.mu.Lock()
	c := &s.core
	change(c)
	s.commitLocked()
	queue := c.queued()
	if !reflect.DeepEqual(queue, c.publishedQueue) {
		c.publishedQueue = queue
		c.dirty = true
		s.eventLocked(&QueueChanged{Queue: queue})
	}
	steers := c.steers()
	if !reflect.DeepEqual(steers, c.publishedSteers) {
		c.publishedSteers = steers
		s.eventLocked(&PendingSteersChanged{PendingSteers: steers})
	}
	phase := c.phase()
	if phase != c.publishedPhase {
		c.publishedPhase = phase
		s.eventLocked(&PhaseChanged{Phase: phase})
	}
	old := c.changed
	c.changed = make(chan struct{})
	status := c.status()
	previous := c.publishedStatus
	if status.Settle != previous.Settle || status.Wakeups != previous.Wakeups || status.AgentInput != previous.AgentInput {
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
	batch := c.log.TakePatches()
	if batch == nil {
		return
	}
	c.rows.Merge(batch.Rows)
	c.dirty = true
	for _, id := range batch.Touched {
		block := c.log.Find(id)
		opens := false
		switch b := block.(type) {
		case *core.UserBlock, *core.ContextBlock:
			opens = true
		case *core.WakeupBlock:
			opens = b.Placement == "new_turn"
		case *core.SteerBlock, *core.AgentMessageBlock, *core.ResumeBlock, *core.AbortBlock, *core.ThinkingBlock, *core.RedactedThinkingBlock, *core.TextBlock, *core.ResponseBlock, *core.ToolCallBlock, *core.ErrorBlock, *core.CompactionBoundaryBlock, *core.CompactionMarkerBlock:
		}
		if opens {
			c.commands.Capture(id, store.BeforeUser, c.commands.Revision())
		}
		c.commands.Capture(id, store.AfterBlock, c.commands.Revision())
	}
	s.eventLocked(&TranscriptChanged{Patches: batch.Patches, Revision: batch.Revision})
}

func (c *coreState) phase() core.SessionPhase {
	if c.stage == Compacting {
		return "compacting"
	}
	if c.active != nil || c.stage == Finalizing || c.interrupted {
		return "running"
	}
	return "idle"
}
func (c *coreState) queued() []core.QueuedMessage {
	queue := []core.QueuedMessage{}
	for _, a := range c.queue {
		if a.kind == sendAction {
			queue = append(queue, core.QueuedMessage{ID: a.turn, Content: a.content})
		}
	}
	return queue
}
func (c *coreState) steers() []core.PendingSteer {
	steers := []core.PendingSteer{}
	for _, input := range c.inputs {
		if input.steer != nil {
			steers = append(steers, *input.steer)
		}
	}
	return steers
}
func (c *coreState) agentInputs() []store.PendingAgentInput {
	inputs := []store.PendingAgentInput{}
	for _, input := range c.inputs {
		if input.agent != nil {
			inputs = append(inputs, *input.agent)
		}
	}
	return inputs
}
func (c *coreState) savedWakeups() []store.ScheduledWakeup {
	wakeups := append([]store.ScheduledWakeup{}, c.wakeups...)
	for _, input := range c.inputs {
		if input.wakeup != nil {
			wakeups = append(wakeups, *input.wakeup)
		}
	}
	return wakeups
}
func (c *coreState) status() Status {
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
func (c *coreState) preparingEdit() bool { return c.edit != nil && !c.edit.accepted }
func (c *coreState) admission() error {
	if c.disposing {
		return AdmissionClosed
	}
	if c.preparingEdit() {
		return AdmissionEditing
	}
	return nil
}
func (c *coreState) steerable() error {
	if c.preparingEdit() {
		return SteerEditing
	}
	if c.stage == Finalizing {
		return SteerFinishing
	}
	if c.active == nil {
		return SteerNotRunning
	}
	if c.active.stopped || c.disposing {
		return SteerStopped
	}
	return nil
}
func (c *coreState) latestSelection() core.ModelSelection {
	if c.waitingChange != nil {
		return c.waitingChange.Model
	}
	if c.change != nil {
		return c.change.Model
	}
	return c.model
}
func (c *coreState) canAbort() bool {
	return !c.disposing && ((c.active != nil && !c.active.stopped && c.stage != Finalizing) || len(c.queue) > 0 || len(c.wakeups) > 0)
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
			_, stopped = blocks[len(blocks)-1].(*core.AbortBlock)
		}
		for _, input := range c.inputs {
			if input.wakeup != nil || (input.agent != nil && !stopped) {
				a = &action{kind: continueAction, turn: core.TurnID(s.deps.IDs.NextID())}
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
	refs := map[core.BlobRef]struct{}{}
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
	for _, steer := range c.steers() {
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

func (c *coreState) removeQueued(id core.TurnID) *action {
	for i, a := range c.queue {
		if a.kind == sendAction && a.turn == id {
			c.queue = slices.Delete(c.queue, i, i+1)
			return a
		}
	}
	return nil
}
