package hostaccess

import (
	"errors"

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
func (e *Error) Error() string {
	switch e.Kind {
	case AccessMissing:
		return "No such conversation"
	case AccessCancelled:
		return "the request went away"
	default:
		if e.Cause != nil {
			return e.Cause.Error()
		}
		return e.Message
	}
}

// Unwrap preserves the underlying failure.
func (e *Error) Unwrap() error {
	return e.Cause
}

// Code returns the response error code and HTTP status.
func (e *Error) Code() (webapi.ErrorCode, int) {
	switch e.Kind {
	case AccessMissing:
		return webapi.ErrorCodeConversationNotFound, 404
	case AccessRefused:
		var refusal Refusal
		if errors.As(e.Cause, &refusal) {
			switch refusal {
			case Archived:
				return webapi.ErrorCodeConversationArchived, 409
			case NotAttached:
				return webapi.ErrorCodeHostNotAttached, 404
			case Busy:
				return webapi.ErrorCodeConversationBusy, 409
			case Stopped:
				return webapi.ErrorCodeHostStopped, 409
			case DeviceGone:
				return webapi.ErrorCodeDeviceNotFound, 404
			}
		}
	case AccessCloud:
		var coded interface {
			Code() (webapi.ErrorCode, int)
		}
		if errors.As(e.Cause, &coded) {
			return coded.Code()
		}
	case AccessHost:
		var failure *host.Error
		if errors.As(e.Cause, &failure) {
			return HostErrorCode(failure)
		}
	}
	return webapi.ErrorCodeInternalError, 500
}

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
func (e *ChangeRefusal) Error() string {
	switch e.Kind {
	case ChangeNotFound:
		return "No such conversation"
	case ChangeArchived:
		return "Restore the conversation before changing it"
	case ChangeTurnInFlight:
		return "A conversation with running work cannot be archived, restored or moved"
	case ChangeProviderNotFound:
		return "No such provider"
	case ChangeModelNotFound:
		return "The provider does not list that model"
	case ChangeModelNotSelected:
		return "Choose a model for the conversation first"
	case ChangeRuntime:
		return "The model cannot run: " + e.Message
	case ChangeWorkspaceNotFound:
		return "No such workspace"
	case ChangeDeviceNotFound:
		return "No such device"
	case ChangeConflict:
		return "Another target change came first"
	case ChangeHostIsMain:
		return "That device is the conversation's main host"
	case ChangeNotAttached:
		return "No such attached host"
	case ChangeNameTaken:
		return "Another attached host has that name"
	default:
		if e.Cause != nil {
			return e.Cause.Error()
		}
		return e.Message
	}
}

// Unwrap preserves the underlying failure.
func (e *ChangeRefusal) Unwrap() error {
	return e.Cause
}

// Code returns the response error code and HTTP status.
func (e *ChangeRefusal) Code() (webapi.ErrorCode, int) {
	switch e.Kind {
	case ChangeNotFound:
		return webapi.ErrorCodeConversationNotFound, 404
	case ChangeArchived:
		return webapi.ErrorCodeConversationArchived, 409
	case ChangeTurnInFlight:
		return webapi.ErrorCodeTurnInFlight, 409
	case ChangeProviderNotFound:
		return webapi.ErrorCodeProviderNotFound, 404
	case ChangeModelNotFound:
		return webapi.ErrorCodeModelNotFound, 404
	case ChangeSettingUnavailable:
		return webapi.ErrorCodeSettingUnavailable, 409
	case ChangeModelNotSelected:
		return webapi.ErrorCodeModelNotSelected, 409
	case ChangeWorkspaceNotFound:
		return webapi.ErrorCodeWorkspaceNotFound, 404
	case ChangeDeviceNotFound:
		return webapi.ErrorCodeDeviceNotFound, 404
	case ChangeConflict:
		return webapi.ErrorCodeTargetConflict, 409
	case ChangeHostIsMain:
		return webapi.ErrorCodeHostIsMain, 409
	case ChangeNotAttached:
		return webapi.ErrorCodeHostNotAttached, 404
	case ChangeNameTaken:
		return webapi.ErrorCodeNameTaken, 409
	}
	return webapi.ErrorCodeOperationFailed, 500
}

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
func (e *RemoteFileRefusal) Error() string {
	switch e.Kind {
	case RemoteFileNotAccessible:
		return "Referenced device is not accessible"
	case RemoteFileOffline:
		return "Referenced device " + e.Message + " is offline"
	case RemoteFileUnquotable:
		return "A referenced path cannot be written in a shell command"
	default:
		if e.Cause != nil {
			return e.Cause.Error()
		}
		return e.Message
	}
}

// Unwrap preserves the underlying failure.
func (e *RemoteFileRefusal) Unwrap() error {
	return e.Cause
}

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
func (e *StreamError) Error() string {
	if e.Cause != nil {
		return e.Cause.Error()
	}
	return e.Message
}

// Unwrap preserves the underlying failure.
func (e *StreamError) Unwrap() error {
	return e.Cause
}

// Code returns the response error code and HTTP status.
func (e *StreamError) Code() (webapi.ErrorCode, int) {
	var access *Error
	if e.Kind == StreamAccess && errors.As(e.Cause, &access) {
		return access.Code()
	}
	return webapi.ErrorCodeStreamFailed, 502
}

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
func (e *UserCallError) Error() string {
	if e.Cause != nil {
		return e.Cause.Error()
	}
	return e.Message
}

// Unwrap preserves the underlying failure.
func (e *UserCallError) Unwrap() error {
	return e.Cause
}

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
func (e *ReadFilesError) Error() string {
	if e.Kind == ReadFilesNotRunning {
		return "the conversation's Host is not running"
	}
	if e.Cause != nil {
		return e.Cause.Error()
	}
	return e.Message
}

// Unwrap preserves the underlying failure.
func (e *ReadFilesError) Unwrap() error {
	return e.Cause
}

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
func (r Refusal) Error() string {
	switch r {
	case Archived:
		return "Conversation is archived"
	case NotAttached:
		return "No such host in this conversation"
	case Busy:
		return "The conversation is changing; its file transfers and streams are closed"
	case Stopped:
		return "The Cloud is stopped"
	case DeviceGone:
		return "The conversation's device no longer exists"
	}
	return ""
}

// HostErrorCode maps Host failures to response codes: offline 409, unavailable
// 503, listing too large 413, missing path 404, permission 403, others 500.
func HostErrorCode(err *host.Error) (webapi.ErrorCode, int) {
	switch err.Kind {
	case host.Offline:
		return webapi.ErrorCodeDeviceOffline, 409
	case host.Unavailable:
		return webapi.ErrorCodeCloudUnavailable, 503
	case host.TooLarge:
		return webapi.ErrorCodeDirectoryTooLarge, 413
	case host.Failed:
		if err.Code != "" {
			switch err.Code {
			case "ENOENT":
				return webapi.ErrorCodeFsError, 404
			case "EACCES", "EPERM":
				return webapi.ErrorCodeFsError, 403
			}
		}
	case host.Protocol, host.Interrupted:
	}
	return webapi.ErrorCodeHostOperationFailed, 500
}
