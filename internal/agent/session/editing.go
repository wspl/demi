package session

// revive:disable:unused-parameter API checkpoint stubs retain parameter names for callers.

import (
	"context"

	"github.com/wspl/demi/internal/agent/store"
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
type Acceptance struct{}

// Wait waits for acceptance, returning the receipt or *EditError. A cancelled
// wait returns its context error; a lost session returns EditClosed.
func (a *Acceptance) Wait(ctx context.Context) (store.EditReceipt, error) {
	panic("not written: a-session")
}

// EditDigest returns the SHA-256 of the request's RFC 8785 canonical JSON,
// before its uploads are resolved. Invalid constructed requests return an error.
func EditDigest(request framewire.EditRequest) (string, error) { panic("not written: a-session") }

// ForkSeed prepares an idle root checkpoint through completed text target,
// with the command state bound to that completion and no waiting work.
func ForkSeed(blocks []core.Block, commands *store.CommandStateHistory, state store.CheckpointState, target core.BlockID) (store.Checkpoint, error) {
	panic("not written: a-session")
}
