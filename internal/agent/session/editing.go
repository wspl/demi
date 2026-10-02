package session

import (
	"context"

	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/framewire"
)

// EditContent is one part of an edit's content as the session receives it.
//
//sumtype:decl
type EditContent interface{ editContent() }

// Content carries a resolved user-content block.
type Content struct{ Block core.UserContentBlock }

// KeptAttachment names the attachment record the edited message holds at Path.
// The session puts that record in the reference's place.
type KeptAttachment struct{ Path string }

// KeptMedia names a native media block of the target message by kind and blob.
type KeptMedia struct{ Media framewire.MediaRef }

func (*Content) editContent()        {}
func (*KeptAttachment) editContent() {}
func (*KeptMedia) editContent()      {}

// EditSubmission is an edit with resolved content and the digest of the request
// as the web app sent it, before uploads were resolved.
type EditSubmission struct {
	OperationID core.OperationID
	Target      core.BlockID
	Version     framewire.TranscriptVersion
	Content     []EditContent
	Digest      string
}

// EditCheck is what the session knows of an operation before its edit runs.
//
//sumtype:decl
type EditCheck interface{ editCheck() }

// EditAccepted carries the receipt of an operation accepted before.
type EditAccepted struct{ Receipt store.EditReceipt }

// EditInFlight shares the acceptance of an operation still being prepared.
type EditInFlight struct{ Acceptance *Acceptance }

// EditProceed means the operation is new and its snapshot current.
type EditProceed struct{}

func (*EditAccepted) editCheck() {}
func (*EditInFlight) editCheck() {}
func (*EditProceed) editCheck()  {}

// Acceptance is an in-flight edit's durable acceptance, shared by every caller
// of the same request. Cancelling a wait does not cancel the admitted edit.
// Successful waits resolve after EditCommitted delivery. To order a reply
// before replacement events, publish it in that event's callback: waking a
// waiter does not serialize the waiter's work with later callbacks.
type Acceptance struct{ result *editResult }

// Wait waits for acceptance, returning the receipt or *EditError. A cancelled
// wait returns its context error; a lost session returns EditClosed.
func (a *Acceptance) Wait(ctx context.Context) (store.EditReceipt, error) {
	return a.wait(ctx)
}

// EditDigest returns the SHA-256 of the request's RFC 8785 canonical JSON,
// before its uploads are resolved. Invalid constructed requests return an error.
func EditDigest(request framewire.EditRequest) (string, error) {
	return commandwire.CanonicalDigest(request)
}

// ForkSeed prepares an idle root checkpoint through completed text target,
// with the command state bound to that completion and no waiting work.
func ForkSeed(blocks []core.Block, commands *store.CommandStateHistory, state store.CheckpointState, target core.BlockID) (store.Checkpoint, error) {
	prefix, err := transcript.ThroughAssistant(blocks, target)
	if err != nil {
		return store.Checkpoint{}, &ForkError{Kind: ForkTarget, Cause: err}
	}
	revision, ok := commands.Boundary(target, store.AfterAssistant)
	if !ok {
		return store.Checkpoint{}, &ForkError{Kind: ForkNoBoundary}
	}
	state.Phase = "idle"
	state.Queue = []core.QueuedMessage{}
	state.AgentInputs = []store.PendingAgentInput{}
	state.Wakeups = []store.ScheduledWakeup{}
	state.Edits = []store.EditReceipt{}
	return store.Checkpoint{State: state, Transcript: prefix, CommandState: commands.Select(prefix, revision, true)}, nil
}
