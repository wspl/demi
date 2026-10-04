package session

import (
	"errors"

	"github.com/wspl/demi/internal/types"
)

//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
var (
	// ErrClosed refuses work after disposal begins.
	ErrClosed = errors.New("The agent session is closed")
	// ErrEditing refuses work while a message edit is being prepared.
	ErrEditing = errors.New("A message edit is being prepared")
	// ErrSteerNotRunning means there is no running turn to steer.
	ErrSteerNotRunning = errors.New("No turn is running to steer")
	// ErrSteerStopped means the running turn is being stopped.
	ErrSteerStopped = errors.New("The running turn is being stopped")
	// ErrSteerFinishing means the running turn is finishing.
	ErrSteerFinishing = errors.New("The running turn is finishing")
	// ErrAgentMessageRecipient means an agent message is addressed to another session.
	ErrAgentMessageRecipient = errors.New("Agent message recipient does not match this session")
	// ErrAgentMessageDifferentContent means the message id already belongs to other content.
	ErrAgentMessageDifferentContent = errors.New("Agent message id already belongs to different content")
	// ErrAgentMessageConflict means the message id conflicts with another input.
	ErrAgentMessageConflict = errors.New("Agent message id conflicts with another input")
	// ErrEditConflict means an edit operation id was reused for another request.
	ErrEditConflict = errors.New("The edit operation ID was used for a different request")
	// ErrEditBusy means the session has pending work.
	ErrEditBusy = errors.New("Message editing requires a settled session with no pending work")
	// ErrEditStale means the editor's transcript version is no longer current.
	ErrEditStale = errors.New("The conversation changed; reopen the message to edit it")
	// ErrEditStopped means the edit stopped before it was accepted.
	ErrEditStopped = errors.New("The edit was stopped before it was accepted")
)

// ReportError is a failure as clients see it: the frame an error event becomes.
type ReportError struct {
	Message     string
	Code        *string
	Diagnostics *types.ProviderErrorDiagnostics
}

// Error returns the message of an action failure.
func (e *ReportError) Error() string {
	return e.Message
}

// ForkError explains why a Fork cannot start where it was asked.
type ForkError struct {
	Kind ForkErrorKind
	// Detail is a store failure's message.
	Detail string
	Cause  error
}

// ForkErrorKind identifies why a Fork's seed could not be built.
type ForkErrorKind uint8

const (
	// ForkTarget means the target cannot be a Fork boundary.
	ForkTarget ForkErrorKind = iota
	// ForkNoBoundary means command history has no boundary after the target.
	ForkNoBoundary
	// ForkNotRoot means the source is not a root session.
	ForkNotRoot
	// ForkNoCheckpoint means no matching source checkpoint exists.
	ForkNoCheckpoint
	// ForkInvalidSeed means the seed is not idle or has a queue or edit receipts.
	ForkInvalidSeed
	// ForkStore means the source's store failed.
	ForkStore
)

// Error returns the user-facing Fork refusal.
func (e *ForkError) Error() string {
	switch e.Kind {
	case ForkTarget:
		if e.Cause != nil {
			return e.Cause.Error()
		}
		return e.Detail
	case ForkNoBoundary:
		return "No command-state boundary after the Fork target"
	case ForkNotRoot:
		return "The Fork source must be a root session"
	case ForkNoCheckpoint:
		return "No matching Fork source checkpoint"
	case ForkInvalidSeed:
		return "A Fork must start idle, without queued actions or edit receipts"
	case ForkStore:
		return e.Detail
	}
	return e.Detail
}

// Unwrap returns the target or store failure, when present.
func (e *ForkError) Unwrap() error {
	return e.Cause
}
