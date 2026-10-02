package hostaccess

//revive:disable:unused-parameter

import (
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapi"
)

// AccessErrorKind identifies the category of Error.
type AccessErrorKind uint8

const (
	// AccessMissing means the user has no conversation of the ID.
	AccessMissing AccessErrorKind = iota
	// AccessRefused means conversation state refused admission.
	AccessRefused
	// AccessCloud means cloud admission failed.
	AccessCloud
	// AccessCancelled means the requester left while admission waited.
	AccessCancelled
	// AccessHost means the Host failed an operation.
	AccessHost
	// AccessStorage means control storage failed.
	AccessStorage
	// AccessObjects means the object store failed.
	AccessObjects
	// AccessStore means the agent blob view failed.
	AccessStore
)

// Error records why conversation Host access admitted no operation or failed one.
// Cause preserves wrapped failures for errors.Is and errors.As. Message is
// additional detail only for variants carrying text in the Rust boundary.
type Error struct {
	Kind    AccessErrorKind
	Message string
	Cause   error
}

// Error returns the original product refusal or underlying failure text.
func (e *Error) Error() string { panic("not written: b-hostaccess") }

// Unwrap preserves the underlying failure.
func (e *Error) Unwrap() error { panic("not written: b-hostaccess") }

// Code returns the response error code and HTTP status.
func (e *Error) Code() (webapi.ErrorCode, int) { panic("not written: b-hostaccess") }

// ChangeErrorKind identifies the category of ChangeRefusal.
type ChangeErrorKind uint8

const (
	// ChangeNotFound means no such conversation.
	ChangeNotFound ChangeErrorKind = iota
	// ChangeArchived means restore the conversation before changing it.
	ChangeArchived
	// ChangeTurnInFlight means running work or conflicting file work holds the conversation.
	ChangeTurnInFlight
	// ChangeProviderNotFound means no such provider.
	ChangeProviderNotFound
	// ChangeModelNotFound means the provider does not list that model.
	ChangeModelNotFound
	// ChangeSettingUnavailable means the model does not support the setting.
	ChangeSettingUnavailable
	// ChangeModelNotSelected means choose a model for the conversation first.
	ChangeModelNotSelected
	// ChangeRuntime means the model cannot run; Message carries the reason.
	ChangeRuntime
	// ChangeWorkspaceNotFound means no such workspace.
	ChangeWorkspaceNotFound
	// ChangeDeviceNotFound means no such paired destination device.
	ChangeDeviceNotFound
	// ChangeConflict means another target change came first.
	ChangeConflict
	// ChangeHostIsMain means that device is the conversation main Host.
	ChangeHostIsMain
	// ChangeNotAttached means no such attached host.
	ChangeNotAttached
	// ChangeNameTaken means another attached host has that name.
	ChangeNameTaken
	// ChangeStorage means control storage failed.
	ChangeStorage
)

// ChangeRefusal records why a conversation change was not applied.
// Cause preserves wrapped failures for errors.Is and errors.As. Message is
// additional detail only for variants carrying text in the Rust boundary.
type ChangeRefusal struct {
	Kind    ChangeErrorKind
	Message string
	Cause   error
}

// Error returns the original product refusal or underlying failure text.
func (e *ChangeRefusal) Error() string { panic("not written: b-hostaccess") }

// Unwrap preserves the underlying failure.
func (e *ChangeRefusal) Unwrap() error { panic("not written: b-hostaccess") }

// Code returns the response error code and HTTP status.
func (e *ChangeRefusal) Code() (webapi.ErrorCode, int) { panic("not written: b-hostaccess") }

// RemoteFileErrorKind identifies the category of RemoteFileRefusal.
type RemoteFileErrorKind uint8

const (
	// RemoteFileNotAccessible means referenced device is not accessible.
	RemoteFileNotAccessible RemoteFileErrorKind = iota
	// RemoteFileOffline means the device named by Message is offline.
	RemoteFileOffline
	// RemoteFileUnquotable means a referenced path cannot be written in a shell command.
	RemoteFileUnquotable
	// RemoteFileStorage means control storage failed.
	RemoteFileStorage
)

// RemoteFileRefusal records why a message remote file reference was refused.
// Cause preserves wrapped failures for errors.Is and errors.As. Message is
// additional detail only for variants carrying text in the Rust boundary.
type RemoteFileRefusal struct {
	Kind    RemoteFileErrorKind
	Message string
	Cause   error
}

// Error returns the original product refusal or underlying failure text.
func (e *RemoteFileRefusal) Error() string { panic("not written: b-hostaccess") }

// Unwrap preserves the underlying failure.
func (e *RemoteFileRefusal) Unwrap() error { panic("not written: b-hostaccess") }

// StreamErrorKind identifies the category of StreamError.
type StreamErrorKind uint8

const (
	// StreamAccess means host admission failed.
	StreamAccess StreamErrorKind = iota
	// StreamFailed means the service failed to start or refused the stream.
	StreamFailed
)

// StreamError records why a user stream did not open.
// Cause preserves wrapped failures for errors.Is and errors.As. Message is
// additional detail only for variants carrying text in the Rust boundary.
type StreamError struct {
	Kind    StreamErrorKind
	Message string
	Cause   error
}

// Error returns the original product refusal or underlying failure text.
func (e *StreamError) Error() string { panic("not written: b-hostaccess") }

// Unwrap preserves the underlying failure.
func (e *StreamError) Unwrap() error { panic("not written: b-hostaccess") }

// Code returns the response error code and HTTP status.
func (e *StreamError) Code() (webapi.ErrorCode, int) { panic("not written: b-hostaccess") }

// UserCallErrorKind identifies the category of UserCallError.
type UserCallErrorKind uint8

const (
	// UserCallAccess means host admission failed.
	UserCallAccess UserCallErrorKind = iota
	// UserCallCall means the native service call failed with its own words.
	UserCallCall
)

// UserCallError records why a one-shot user call failed.
// Cause preserves wrapped failures for errors.Is and errors.As. Message is
// additional detail only for variants carrying text in the Rust boundary.
type UserCallError struct {
	Kind    UserCallErrorKind
	Message string
	Cause   error
}

// Error returns the original product refusal or underlying failure text.
func (e *UserCallError) Error() string { panic("not written: b-hostaccess") }

// Unwrap preserves the underlying failure.
func (e *UserCallError) Unwrap() error { panic("not written: b-hostaccess") }

// ReadFilesErrorKind identifies the category of ReadFilesError.
type ReadFilesErrorKind uint8

const (
	// ReadFilesNotRunning means the Host is not running, and a read never wakes it.
	ReadFilesNotRunning ReadFilesErrorKind = iota
	// ReadFilesAccess means host admission or reading failed.
	ReadFilesAccess
)

// ReadFilesError records why a read of conversation Host files did not answer.
// Cause preserves wrapped failures for errors.Is and errors.As. Message is
// additional detail only for variants carrying text in the Rust boundary.
type ReadFilesError struct {
	Kind    ReadFilesErrorKind
	Message string
	Cause   error
}

// Error returns the original product refusal or underlying failure text.
func (e *ReadFilesError) Error() string { panic("not written: b-hostaccess") }

// Unwrap preserves the underlying failure.
func (e *ReadFilesError) Unwrap() error { panic("not written: b-hostaccess") }

// Refusal is what the conversation's state refuses.
type Refusal uint8

const (
	// Archived refuses Host access for an archived conversation.
	Archived Refusal = iota
	// NotAttached refuses a device outside the main and attached bindings.
	NotAttached
	// Busy means a transition is closing file transfers and streams.
	Busy
	// Stopped means the Cloud is stopped and this operation never wakes it.
	Stopped
	// DeviceGone means the conversation's device no longer exists.
	DeviceGone
)

// Error returns the verbatim conversation-state refusal.
func (r Refusal) Error() string { panic("not written: b-hostaccess") }

// HostErrorCode maps Host failures to response codes: offline 409, unavailable
// 503, listing too large 413, missing path 404, permission 403, others 500.
func HostErrorCode(err *host.Error) (webapi.ErrorCode, int) { panic("not written: b-hostaccess") }
