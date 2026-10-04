package database

import (
	"github.com/wspl/demi/internal/machineproto"
	"github.com/wspl/demi/internal/webapiproto"
)

// ManagedOperation is a reset of a Cloud, as its intent is kept: the base it selected when it
// was admitted, the phase it reached, and why it failed. A retry resumes
// the same operation on the same base.
type ManagedOperation struct {
	ID          webapiproto.OperationID
	BaseVersion machineproto.BaseVersion
	Phase       webapiproto.ResetPhase
	Error       *string
}

// CloudUseRecord describes one of the user's conversations as the Cloud's lifecycle weighs it
// (`sessions-and-targets.md` § How a conversation uses a device): whether
// its files and commands are on the Cloud, the provider entry it infers
// with, and whether the Cloud is attached to it.
type CloudUseRecord struct {
	ID       webapiproto.ConversationID
	OnCloud  bool
	Provider *webapiproto.ProviderID
	Attached bool
}
