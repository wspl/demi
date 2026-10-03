package session

import (
	"context"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
)

// TranscriptSnapshot is the transcript at one moment, with its version.
// The blocks are detached from mutable session state.
type TranscriptSnapshot struct {
	Blocks  []core.Block
	Version framewire.TranscriptVersion
}

// Snapshot is the client-visible state at one session decision. Its values
// are detached or immutable, like the corresponding individual reads.
type Snapshot struct {
	Transcript    TranscriptSnapshot
	Phase         core.SessionPhase
	Queue         []core.QueuedMessage
	PendingSteers []core.PendingSteer
}

// ActionEnd says how an action ended when it did not fail.
type ActionEnd uint8

const (
	// Completed means the action finished successfully.
	Completed ActionEnd = iota
	// Aborted means the action was stopped and recorded the stop.
	Aborted
	// Dropped means the action left the queue without running.
	Dropped
	// Detached means dispose kept the action queued in the final checkpoint.
	Detached
	// Duplicate means the session already knew the message's id.
	Duplicate
)

// ActionHandle resolves when an admitted action ends. Abandoning the handle or
// cancelling a wait does not stop the action; Abort stops it explicitly.
type ActionHandle struct{ result *actionResult }

// Wait waits for the action's result. An action failure returns *ErrorReport;
// a cancelled wait returns the context error. Multiple waiters share the result.
func (a *ActionHandle) Wait(ctx context.Context) (ActionEnd, error) { return a.wait(ctx) }

// Event is a change reported once complete, in commit order outside the state
// lock. Its data remains immutable for every listener.
//
//sumtype:decl
type Event interface{ sessionEvent() }

// TranscriptChanged carries the patches of one committed transcript change.
type TranscriptChanged struct {
	Patches  []framewire.TranscriptPatch
	Revision uint64
}

// EditCommitted publishes a new edit's durable receipt immediately after its
// rewrite and before replacement progress. The server sends EditResult to the
// initiating connections from this callback, not when CheckEdit or EditAndSend returns.
// A duplicate request returns its receipt without publishing another event.
type EditCommitted struct{ Receipt store.EditReceipt }

func (*EditCommitted) sessionEvent() {}

// PhaseChanged reports the client-visible session phase.
type PhaseChanged struct{ Phase core.SessionPhase }

// QueueChanged reports the complete visible message queue.
type QueueChanged struct{ Queue []core.QueuedMessage }

// PendingSteersChanged reports the human steers waiting for a boundary.
type PendingSteersChanged struct{ PendingSteers []core.PendingSteer }

// RetryScheduled reports a transient provider failure retried after DelayMS.
type RetryScheduled struct {
	Attempt     uint32
	DelayMS     uint64
	Code        *string
	Diagnostics *core.ProviderErrorDiagnostics
}

// ErrorEvent reports a failed turn or a failed save.
type ErrorEvent struct{ Report ErrorReport }

// ActionFailed reports an action that failed for good, after its checkpoint save.
type ActionFailed struct{ Report ErrorReport }

func (*TranscriptChanged) sessionEvent()    {}
func (*PhaseChanged) sessionEvent()         {}
func (*QueueChanged) sessionEvent()         {}
func (*PendingSteersChanged) sessionEvent() {}
func (*RetryScheduled) sessionEvent()       {}
func (*ErrorEvent) sessionEvent()           {}
func (*ActionFailed) sessionEvent()         {}

// Subscription owns an event listener. Its owner defers Release.
type Subscription struct {
	session *Session
	id      uint64
}

// Release ends the subscription, idempotently. It does not join a callback
// already being delivered and is safe to call from that callback.
func (s *Subscription) Release() { s.session.mutate(func(c *coreState) { delete(c.listeners, s.id) }) }

// Settle says whether the session will do anything more by itself.
type Settle uint8

const (
	// Busy means an action runs or waits.
	Busy Settle = iota
	// Settled means no action runs or waits.
	Settled
	// Closed means disposed: nothing will ever run again.
	Closed
)

// Status is an immutable snapshot of whether a session acts and what could
// make it act later. Read the predicate, then wait on Changed and reload Status.
type Status struct {
	changed chan struct{}
	Settle  Settle
	// Wakeups means a yield wakeup is scheduled, or fired and not yet written.
	Wakeups bool
	// AgentInput means an agent message waits for a boundary.
	AgentInput bool
}

// Changed closes after a newer status is published. Notifications may coalesce.
// The snapshot and its notification are captured together, so no wake is lost.
func (s Status) Changed() <-chan struct{} { return s.changed }
