package session

import (
	"context"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
	"github.com/wspl/demi/internal/provider"
)

// Deps supplies the node runtime, checkpoint store, identities, clock and policy.
// Runtime and Store support concurrent calls; the session serializes IDs and
// Clock calls with its state. The caller supplies every dependency explicitly.
type Deps struct {
	Runtime Runtime
	Store   store.SessionStore
	IDs     transcript.IDs
	Clock   core.Clock
	Config  Config
}

// Init is a new session's state. Runtime transfers to the constructed session.
type Init struct {
	ID      core.NodeID
	CWD     string
	Model   core.ModelSelection
	Runtime provider.Runtime
}

// Continuation is what restoration hands back for the node's lifecycle policy.
type Continuation struct {
	// Interrupted means the checkpoint says a turn was running when the
	// process died or dispose stopped it.
	Interrupted bool
	// Queued holds the checkpoint's queued messages in order. The node
	// decides when to submit them to the restored session.
	Queued []core.QueuedMessage
}

// ModelSwitch changes the selection at the next provider request.
type ModelSwitch struct {
	Model core.ModelSelection
	// Runtime is a new runtime when the model belongs to a different
	// provider than the selection before it; nil otherwise.
	Runtime provider.Runtime
}

// Discard closes the runtime of a switch that no session took. The caller
// retains ownership when UpdateModel refuses the switch.
func (s *ModelSwitch) Discard(ctx context.Context) error {
	if s.Runtime == nil {
		return nil
	}
	runtime := s.Runtime
	s.Runtime = nil
	return runtime.Close(ctx)
}

// Session is a handle over one agent's state. Share its pointer; methods are
// safe for concurrent calls. It owns and joins its worker, persister and wakeup
// driver in Dispose. No callback, IO or wait runs under its short state lock.
// Values returned to callers are detached or immutable.
type Session struct{ *sessionOwner }

// New creates a session with an empty transcript. Its node's first checkpoint
// is already in the store. The caller owns the session and must call Dispose.
func New(init Init, deps Deps) *Session {
	return construct(init, deps, transcript.NewLog(nil, deps.IDs, deps.Clock), store.NewCommandStateHistory(), nil)
}

// Restore creates a session from a decoded checkpoint. Executing tool calls
// become interrupted without running again. The session starts idle with
// wakeups armed; input of an interrupted turn waits for the node's next action.
// Success transfers runtime ownership; failure leaves it with the caller.
func Restore(checkpoint store.Checkpoint, id core.NodeID, runtime provider.Runtime, deps Deps) (*Session, Continuation, error) {
	return restore(checkpoint, id, runtime, deps)
}

// FirstCheckpoint returns a new node's state and empty initial command state,
// with no blocks.
func (s *Session) FirstCheckpoint() store.CheckpointUpdate {
	s.mu.Lock()
	defer s.mu.Unlock()
	return store.CheckpointUpdate{State: s.core.checkpointState(), CommandState: new(s.core.commands.Snapshot(nil)), ChangedBlocks: []store.ChangedBlock{}, BlockCount: 0}
}

// ID returns the session's node identity.
func (s *Session) ID() core.NodeID {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.core.id
}

// Phase returns the phase clients see, including running during finalization.
func (s *Session) Phase() core.SessionPhase {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.core.phase()
}

// IsSettled reports whether no action runs or waits.
func (s *Session) IsSettled() bool {
	return s.Status().Settle != Busy
}

// Settled waits until no action runs or waits. Cancellation stops only the wait.
func (s *Session) Settled(ctx context.Context) error {
	return s.waitSettled(ctx)
}

// Model returns the selection current now.
func (s *Session) Model() core.ModelSelection {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.core.model
}

// Transcript returns the blocks and version of one snapshot.
func (s *Session) Transcript() TranscriptSnapshot {
	s.mu.Lock()
	defer s.mu.Unlock()
	return TranscriptSnapshot{Blocks: s.core.log.Blocks(), Version: s.core.log.Version()}
}

// QueuedMessages returns messages in the order they will run.
func (s *Session) QueuedMessages() []core.QueuedMessage {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.core.queued()
}

// PendingSteers returns the human steers accepted and not yet written.
func (s *Session) PendingSteers() []core.PendingSteer {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.core.steers()
}

// LastAssistantText returns the last text block's text, the result a child closes with.
func (s *Session) LastAssistantText() string {
	return transcript.LastAssistantText(s.Transcript().Blocks, 0)
}

// HoldMedia takes the bytes resolved by the backend for the next send, steer or
// edit. Bytes unreferenced by the next request are released.
func (s *Session) HoldMedia(media *store.HeldMedia) {
	s.mutate(func(c *coreState) { c.media.Absorb(*media) })
}

// Send submits a message, queuing it when busy. A known id returns Duplicate
// without creating another turn. Refusals return AdmissionError.
func (s *Session) Send(content []core.UserContentBlock, id core.TurnID) (*ActionHandle, error) {
	return s.admit(sendAction, content, id)
}

// Retry rewinds the last input turn and runs it again.
func (s *Session) Retry() (*ActionHandle, error) {
	return s.admit(retryAction, nil, "")
}

// Resume unwinds the unfinished turn to its resume point and continues it.
func (s *Session) Resume() (*ActionHandle, error) {
	return s.admit(resumeAction, nil, "")
}

// Compact runs one compaction pass.
func (s *Session) Compact() (*ActionHandle, error) {
	return s.admit(compactAction, nil, "")
}

// DequeueMessage removes the named message, returning whether it was queued.
func (s *Session) DequeueMessage(id core.TurnID) bool {
	removed := false
	s.mutate(func(c *coreState) {
		if c.disposing {
			return
		}
		a := c.removeQueued(id)
		if a != nil {
			removed = true
			c.effects = append(c.effects, func() { a.answer.finish(Dropped, nil) })
		}
	})
	return removed
}

// SendQueuedMessage moves the named message to the front of the queue.
func (s *Session) SendQueuedMessage(id core.TurnID) bool {
	moved := false
	s.mutate(func(c *coreState) {
		if c.disposing {
			return
		}
		a := c.removeQueued(id)
		if a == nil {
			return
		}
		i := 0
		for i < len(c.queue) && c.queue[i].kind != sendAction {
			i++
		}
		c.queue = append(c.queue, nil)
		copy(c.queue[i+1:], c.queue[i:])
		c.queue[i] = a
		moved = true
	})
	return moved
}

// ClearMessageQueue removes all queued messages and returns their count.
func (s *Session) ClearMessageQueue() int {
	count := 0
	s.mutate(func(c *coreState) {
		if c.disposing {
			return
		}
		kept := []*action{}
		for _, a := range c.queue {
			if a.kind != sendAction {
				kept = append(kept, a)
				continue
			}
			count++
			c.effects = append(c.effects, func() { a.answer.finish(Dropped, nil) })
		}
		c.queue = kept
	})
	return count
}

// Steer adds input to the running turn at its next boundary. A refusal returns
// SteerError without changing input.
func (s *Session) Steer(content []core.UserContentBlock, id core.BlockID) error {
	return s.steer(content, id)
}

// CancelPendingSteer withdraws a pending steer. An id not pending changes nothing.
func (s *Session) CancelPendingSteer(id core.BlockID) bool {
	return s.cancelSteer(id)
}

// SteerQueuedMessage turns a queued message into a steer, preserving its queue
// position on refusal. It returns false when no queued message has that id.
func (s *Session) SteerQueuedMessage(message core.TurnID, steer core.BlockID) (bool, error) {
	var err error
	found := false
	s.mutate(func(c *coreState) {
		if err = c.steerable(); err != nil {
			return
		}
		a := c.removeQueued(message)
		if a == nil {
			return
		}
		found = true
		c.effects = append(c.effects, func() { a.answer.finish(Dropped, nil) })
		c.inputs = append(c.inputs, pendingInput{steer: &core.PendingSteer{ID: steer, TurnID: c.active.turn, Model: c.model, Content: a.content}})
		c.arrivals++
	})
	return found, err
}

// AcceptAgentMessage admits another agent's message and returns once its
// admission is saved. Failures return *AgentMessageError. A save that starts
// finishes even if ctx is cancelled.
func (s *Session) AcceptAgentMessage(ctx context.Context, message core.AgentMessage) error {
	return s.acceptAgent(ctx, message)
}

// CheckEdit checks an edit before its uploads are resolved, so repeated
// requests write no file. A refusal returns *EditError.
func (s *Session) CheckEdit(operation core.OperationID, digest string, version framewire.TranscriptVersion) (EditCheck, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.core.checkEdit(operation, digest, version)
}

// EditAndSend replaces a user message and everything after it. It returns at
// durable acceptance; the replacement turn runs on. An accepted operation
// returns its receipt again. Cancelling the caller's wait does not stop the
// admitted edit; Abort does. A rejection returns *EditError.
func (s *Session) EditAndSend(ctx context.Context, submission EditSubmission) (store.EditReceipt, error) {
	return s.editAndSend(ctx, submission)
}

// PrepareFork captures a seed through completed text target, with the model a
// switch already accepted by the session would use. Failures return *ForkError.
func (s *Session) PrepareFork(target core.BlockID) (store.Checkpoint, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	state := s.core.checkpointState()
	state.Model = s.core.latestSelection()
	return ForkSeed(s.core.log.Blocks(), s.core.commands, state, target)
}

// Wake opens a continuation for waiting input while nothing runs. The node's
// policy calls it when a restored session may act.
func (s *Session) Wake() {
	s.mutate(func(_ *coreState) { s.startNextLocked() })
}

// Hold keeps waiting input from opening a continuation until another action
// starts, so a closing child can save later deliveries without running again.
func (s *Session) Hold() {
	s.mutate(func(c *coreState) { c.held = true })
}

// Abort stops the running action, else the first waiting action, else the
// oldest wakeup. It waits for the stopped action's acknowledgement. Cancelling
// the wait does not retract the stop; the result describes the acknowledgement.
func (s *Session) Abort(ctx context.Context) (framewire.AbortResult, error) {
	return s.stop(ctx, false)
}

// StopRunning stops the running action and waits until it records the stop.
// Waiting actions remain and run when admission permits them.
func (s *Session) StopRunning(ctx context.Context) error {
	_, err := s.stop(ctx, true)
	return err
}

// UpdateModel records a switch for the next provider request, waiting behind
// edit preparation if needed. Success transfers the runtime to the session.
// AdmissionClosed leaves ownership with the caller, who must Discard it.
func (s *Session) UpdateModel(change ModelSwitch) error {
	var err error
	s.mutate(func(c *coreState) {
		if c.disposing {
			err = AdmissionClosed
			return
		}
		slot := &c.change
		if c.preparingEdit() {
			slot = &c.waitingChange
		}
		c.replaceSwitchLocked(slot, change)
	})
	return err
}

// NeedsRuntimeFor reports whether model belongs to a different provider than
// the selection the next request would use.
func (s *Session) NeedsRuntimeFor(model core.ModelSelection) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.core.latestSelection().ProviderID != model.ProviderID
}

// ForkRuntime waits for the current runtime and returns a fresh one with the
// same configuration and no execution state. The caller owns and closes it.
func (s *Session) ForkRuntime(ctx context.Context) (provider.Runtime, error) {
	return s.forkRuntime(ctx)
}

// RecordInterruption appends the interruption record of a turn the process
// died in, unless already present. Flush saves it.
func (s *Session) RecordInterruption() {
	s.mutate(func(c *coreState) {
		if !c.log.EndsWithInterruption() {
			c.log.PushError(c.model, transcript.InterruptedTurnMessage, new(transcript.InterruptedCode), nil)
		}
	})
}

// Flush saves now, after earlier saves in the session's order. Cancellation
// can end admission waits; a save that starts always finishes.
func (s *Session) Flush(ctx context.Context) error {
	permit, err := s.persist.Acquire(ctx)
	if err != nil {
		return err
	}
	defer permit.Release()
	return s.save(ctx, nil, store.CommitGuard{})
}

// Dispose refuses new actions, stops a running one as shutdown, keeps its queue
// and wakeups, saves the final checkpoint and closes all runtimes. It cancels
// and joins every owned goroutine even when saving fails. Cleanup and commits
// finish despite cancellation; call it with the owner's cleanup context.
func (s *Session) Dispose(ctx context.Context) error {
	return s.dispose(ctx)
}

// Subscribe calls listener with subsequent events until Release. A listener
// may call back into the session; resulting events follow the current event's
// delivery to every listener. Callbacks must not wait for session progress.
func (s *Session) Subscribe(listener func(Event)) *Subscription {
	var id uint64
	s.mutate(func(c *coreState) {
		id = c.nextListener
		c.nextListener++
		c.listeners[id] = listener
	})
	return &Subscription{session: s, id: id}
}

// Status returns the current immutable status with its change notification.
func (s *Session) Status() Status {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.core.publishedStatus
}

// Execution returns what a supervisor observes the session doing now.
func (s *Session) Execution() Execution {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.core.active == nil && s.core.stage != Finalizing && len(s.core.wakeups) > 0 {
		return PendingYield
	}
	if s.core.stage == preparing {
		return ProviderStreaming
	}
	return s.core.stage
}

// IsTextAt reports whether the block at index is assistant text.
func (s *Session) IsTextAt(index int) bool {
	snapshot := s.Transcript()
	if index < 0 || index >= len(snapshot.Blocks) {
		return false
	}
	_, ok := snapshot.Blocks[index].(*core.TextBlock)
	return ok
}
