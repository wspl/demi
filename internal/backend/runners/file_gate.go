package runners

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/webapi"
)

// FileGate is the file gate of one conversation. Construct it with NewFileGate;
// its owner is that conversation's slot in host access. Do not copy it.
type FileGate struct{}

// NewFileGate makes the file gate of conversation, as its record spells the ID.
func NewFileGate(conversation webapi.ConversationID) *FileGate { panic("not written: b-runners") }

// Enter takes a lease for purpose once no reservation holds the gate.
// Only host access enters it; its operation must defer the lease's Release.
func (g *FileGate) Enter(ctx context.Context, purpose gates.Purpose) (*FileLease, error) {
	panic("not written: b-runners")
}

// Gate is the activity gate transitions reserve and idle watches read.
// A lease taken directly from it names no conversation.
func (g *FileGate) Gate() *gates.Activity { panic("not written: b-runners") }

// FileLease is a lease of one conversation's file gate. Only FileGate.Enter
// creates a usable lease. Do not copy it; its owner must release it.
type FileLease struct{}

// Conversation returns the conversation whose file gate this lease holds.
func (l *FileLease) Conversation() webapi.ConversationID { panic("not written: b-runners") }

// Release ends the file lease exactly once. Hosts made against it must no longer
// be used after release.
func (l *FileLease) Release() { panic("not written: b-runners") }
