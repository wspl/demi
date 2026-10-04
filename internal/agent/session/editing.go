package session

import (
	"github.com/wspl/demi/internal/agent/store"
	"github.com/wspl/demi/internal/agent/transcript"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/conversationproto"
	"github.com/wspl/demi/internal/types"
)

// EditContent is one part of an edit's content as the session receives it.
//
//sumtype:decl
type EditContent interface{ editContent() }

// Content carries a resolved user-content block.
type Content struct{ Block types.UserContentBlock }

// KeptAttachment names the attachment record the edited message holds at Path.
// The session puts that record in the reference's place.
type KeptAttachment struct{ Path string }

// KeptMedia names a native media block of the target message by kind and blob.
type KeptMedia struct{ Media conversationproto.MediaRef }

func (*Content) editContent()        {}
func (*KeptAttachment) editContent() {}
func (*KeptMedia) editContent()      {}

// EditSubmission is an edit with resolved content and the digest of the request
// as the web app sent it, before uploads were resolved.
type EditSubmission struct {
	OperationID types.OperationID
	Target      types.BlockID
	Version     conversationproto.TranscriptVersion
	Content     []EditContent
	Digest      string
}

// EditCheck is what the session knows of an operation before its edit runs.
//
//sumtype:decl
type EditCheck interface{ editCheck() }

// EditAccepted carries the receipt of an operation accepted before.
type EditAccepted struct{ Receipt store.EditReceipt }

// EditInFlight means an earlier request was preparing the operation. CheckEdit
// returns it once that edit is accepted, after its EditCommitted event, with
// the acceptance's receipt.
type EditInFlight struct {
	Receipt    store.EditReceipt
	acceptance *acceptance
}

// EditProceed means the operation is new and its snapshot current.
type EditProceed struct{}

func (*EditAccepted) editCheck() {}
func (*EditInFlight) editCheck() {}
func (*EditProceed) editCheck()  {}

// acceptance is an in-flight edit's durable acceptance, shared by every request
// of the same operation. Cancelling a wait does not cancel the admitted edit.
// A successful wait resolves after EditCommitted delivery. To order a reply
// before replacement events, publish it in that event's callback: waking a
// waiter does not serialize the waiter's work with later callbacks.
type acceptance struct{ result *editResult }

// EditDigest returns the SHA-256 of the request's RFC 8785 canonical JSON,
// before its uploads are resolved. Invalid constructed requests return an error.
func EditDigest(request conversationproto.EditRequest) (string, error) {
	return commandproto.CanonicalDigest(request)
}

// ForkSeed prepares an idle root checkpoint through completed text target,
// with the command state bound to that completion and no waiting work.
func ForkSeed(
	blocks []types.Block,
	commands *store.CommandStateHistory,
	state store.CheckpointState,
	target types.BlockID,
) (store.Checkpoint, error) {
	prefix, err := transcript.ThroughAssistant(blocks, target)
	if err != nil {
		return store.Checkpoint{}, &ForkError{Kind: ForkTarget, Cause: err}
	}
	revision, ok := commands.Boundary(target, store.AfterAssistant)
	if !ok {
		return store.Checkpoint{}, &ForkError{Kind: ForkNoBoundary}
	}
	state.Phase = "idle"
	state.Queue = []types.QueuedMessage{}
	state.AgentInputs = []store.PendingAgentInput{}
	state.Wakeups = []store.ScheduledWakeup{}
	state.Edits = []store.EditReceipt{}
	return store.Checkpoint{
		State:        state,
		Transcript:   prefix,
		CommandState: commands.Select(prefix, revision, true),
	}, nil
}
