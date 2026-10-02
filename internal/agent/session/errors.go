package session

import "github.com/wspl/demi/internal/core"

// AdmissionError explains why a session refused an action or a change.
// Compare it with errors.Is.
type AdmissionError string

const (
	// AdmissionClosed refuses work after disposal begins.
	AdmissionClosed AdmissionError = "The agent session is closed"
	// AdmissionEditing refuses work during edit preparation.
	AdmissionEditing AdmissionError = "A message edit is being prepared"
)

// Error returns the admission refusal's user-facing text.
func (e AdmissionError) Error() string { panic("not written: a-session") }

// SteerError explains why a steer was refused. Compare it with errors.Is.
type SteerError string

const (
	// SteerNotRunning means there is no running turn.
	SteerNotRunning SteerError = "No turn is running to steer"
	// SteerStopped means the running turn is being stopped.
	SteerStopped SteerError = "The running turn is being stopped"
	// SteerFinishing means the running turn is finishing.
	SteerFinishing SteerError = "The running turn is finishing"
	// SteerEditing means an edit is being prepared.
	SteerEditing SteerError = "A message edit is being prepared"
)

// Error returns the steer's user-facing refusal.
func (e SteerError) Error() string { panic("not written: a-session") }

// AgentMessageError explains why an agent message was refused. Use errors.As
// to inspect Kind and errors.Is to inspect an underlying validation or store error.
type AgentMessageError struct {
	Kind AgentMessageErrorKind
	// Detail is the validation report for AgentMessageInvalid.
	Detail string
	Cause  error
}

// AgentMessageErrorKind identifies an agent-message admission refusal.
type AgentMessageErrorKind uint8

const (
	// AgentMessageRecipient means the recipient does not match the session.
	AgentMessageRecipient AgentMessageErrorKind = iota
	// AgentMessageDifferentContent means the id already belongs to other content.
	AgentMessageDifferentContent
	// AgentMessageConflict means the id conflicts with another input.
	AgentMessageConflict
	// AgentMessageInvalid means the message failed validation.
	AgentMessageInvalid
	// AgentMessageClosed means the session is closed.
	AgentMessageClosed
	// AgentMessageEditing means an edit is being prepared.
	AgentMessageEditing
	// AgentMessageStore means admission is not durable because its save failed.
	AgentMessageStore
)

// Error returns the user-facing agent-message refusal.
func (e *AgentMessageError) Error() string { panic("not written: a-session") }

// Unwrap returns the validation or store error, when present.
func (e *AgentMessageError) Unwrap() error { panic("not written: a-session") }

// RestoreError explains why a checkpoint could not be restored.
type RestoreError struct {
	Kind RestoreErrorKind
	// Detail is the invalid waiting-input reason or repeated edit operation id.
	Detail string
	Cause  error
}

// RestoreErrorKind identifies the checkpoint invariant that failed.
type RestoreErrorKind uint8

const (
	// RestoreCommandState means command-state history is invalid.
	RestoreCommandState RestoreErrorKind = iota
	// RestoreInput means the checkpoint's waiting input is invalid.
	RestoreInput
	// RestoreEdits means the checkpoint repeats an edit operation id.
	RestoreEdits
)

// Error returns the user-facing restoration failure.
func (e *RestoreError) Error() string { panic("not written: a-session") }

// Unwrap returns the underlying command-state or input error.
func (e *RestoreError) Unwrap() error { panic("not written: a-session") }

// ErrorReport is a failure as clients see it: the frame an error event becomes.
type ErrorReport struct {
	Message     string
	Code        *string
	Diagnostics *core.ProviderErrorDiagnostics
}

// Error returns the message of an action failure.
func (e *ErrorReport) Error() string { panic("not written: a-session") }

// EditError explains why an edit was rejected. Nothing of the history changed.
// Use errors.As to inspect Kind and errors.Is for the underlying cause.
type EditError struct {
	Kind EditErrorKind
	// Detail is the unknown attachment path or preparation failure message.
	Detail string
	// MediaKind and Blob identify an unknown kept medium.
	MediaKind string
	Blob      core.BlobRef
	Cause     error
}

// EditErrorKind identifies why an edit could not be accepted.
type EditErrorKind uint8

const (
	// EditConflict means an operation id was reused for another request.
	EditConflict EditErrorKind = iota
	// EditBusy means the session has pending work.
	EditBusy
	// EditStale means the editor's transcript version is no longer current.
	EditStale
	// EditTarget means the target is not an editable user message.
	EditTarget
	// EditUnknownAttachment means the target holds no attachment at the path.
	EditUnknownAttachment
	// EditUnknownMedia means the target holds no matching native medium.
	EditUnknownMedia
	// EditStopped means the edit stopped before acceptance.
	EditStopped
	// EditClosed means the session is closed.
	EditClosed
	// EditFailed means preparation, saving, or node admission failed.
	EditFailed
)

// Error returns the user-facing edit refusal.
func (e *EditError) Error() string { panic("not written: a-session") }

// Unwrap returns the target, preparation or persistence failure, when present.
func (e *EditError) Unwrap() error { panic("not written: a-session") }

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
func (e *ForkError) Error() string { panic("not written: a-session") }

// Unwrap returns the target or store failure, when present.
func (e *ForkError) Unwrap() error { panic("not written: a-session") }
