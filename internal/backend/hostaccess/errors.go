package hostaccess

import (
	"errors"

	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/webapiproto"
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
	// AccessStorage means control storage, the object store or the agent blob view failed.
	AccessStorage
)

// Error records why conversation Host access admitted no operation or failed one.
// Cause preserves wrapped failures for errors.Is and errors.As.
type Error struct {
	Kind  AccessErrorKind
	Cause error
}

// Error returns the original product refusal or underlying failure text.
func (e *Error) Error() string {
	switch e.Kind {
	case AccessMissing:
		return "No such conversation"
	case AccessCancelled:
		return "the request went away"
	default:
		return e.Cause.Error()
	}
}

// Unwrap preserves the underlying failure.
func (e *Error) Unwrap() error {
	return e.Cause
}

// Code returns the response error code and HTTP status.
func (e *Error) Code() (webapiproto.ErrorCode, int) {
	switch e.Kind {
	case AccessMissing:
		return webapiproto.ErrorCodeConversationNotFound, 404
	case AccessRefused:
		var refusal Refusal
		if errors.As(e.Cause, &refusal) {
			switch refusal {
			case Archived:
				return webapiproto.ErrorCodeConversationArchived, 409
			case NotAttached:
				return webapiproto.ErrorCodeHostNotAttached, 404
			case Busy:
				return webapiproto.ErrorCodeConversationBusy, 409
			case Stopped:
				return webapiproto.ErrorCodeHostStopped, 409
			case DeviceGone:
				return webapiproto.ErrorCodeDeviceNotFound, 404
			}
		}
	case AccessCloud:
		var coded interface {
			Code() (webapiproto.ErrorCode, int)
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
	return webapiproto.ErrorCodeInternalError, 500
}

// ChangeErrorKind identifies the category of ChangeError.
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
	// ChangeRuntime means the model cannot run; Cause says why.
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

// ChangeError records why a conversation change was not applied.
// Cause preserves wrapped failures for errors.Is and errors.As.
type ChangeError struct {
	Kind  ChangeErrorKind
	Cause error
}

// Error returns the original product refusal or underlying failure text.
func (e *ChangeError) Error() string {
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
		return "The model cannot run: " + e.Cause.Error()
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
		return e.Cause.Error()
	}
}

// Unwrap preserves the underlying failure.
func (e *ChangeError) Unwrap() error {
	return e.Cause
}

// Code returns the response error code and HTTP status.
func (e *ChangeError) Code() (webapiproto.ErrorCode, int) {
	switch e.Kind {
	case ChangeNotFound:
		return webapiproto.ErrorCodeConversationNotFound, 404
	case ChangeArchived:
		return webapiproto.ErrorCodeConversationArchived, 409
	case ChangeTurnInFlight:
		return webapiproto.ErrorCodeTurnInFlight, 409
	case ChangeProviderNotFound:
		return webapiproto.ErrorCodeProviderNotFound, 404
	case ChangeModelNotFound:
		return webapiproto.ErrorCodeModelNotFound, 404
	case ChangeSettingUnavailable:
		return webapiproto.ErrorCodeSettingUnavailable, 409
	case ChangeModelNotSelected:
		return webapiproto.ErrorCodeModelNotSelected, 409
	case ChangeWorkspaceNotFound:
		return webapiproto.ErrorCodeWorkspaceNotFound, 404
	case ChangeDeviceNotFound:
		return webapiproto.ErrorCodeDeviceNotFound, 404
	case ChangeConflict:
		return webapiproto.ErrorCodeTargetConflict, 409
	case ChangeHostIsMain:
		return webapiproto.ErrorCodeHostIsMain, 409
	case ChangeNotAttached:
		return webapiproto.ErrorCodeHostNotAttached, 404
	case ChangeNameTaken:
		return webapiproto.ErrorCodeNameTaken, 409
	}
	return webapiproto.ErrorCodeOperationFailed, 500
}

// ErrDeviceNotAccessible refuses a reference to a device that is not the user's.
//
//nolint:staticcheck // ST1005: product text, shown to the user as it is.
var ErrDeviceNotAccessible = errors.New("Referenced device is not accessible")

// ErrPathUnquotable refuses a path that cannot be written in a shell command.
//
//nolint:staticcheck // ST1005: product text, shown to the user as it is.
var ErrPathUnquotable = errors.New("A referenced path cannot be written in a shell command")

// StreamError is a user stream whose service failed to start or refused it.
type StreamError struct{ Err error }

// Error returns the underlying failure text.
func (e *StreamError) Error() string { return e.Err.Error() }

// Unwrap preserves the underlying failure.
func (e *StreamError) Unwrap() error { return e.Err }

// Code returns stream_failed with status 502.
func (e *StreamError) Code() (webapiproto.ErrorCode, int) {
	return webapiproto.ErrorCodeStreamFailed, 502
}

// streamAccessError keeps an admission failure's own code; any other failure is a failed stream.
func streamAccessError(err error) error {
	var access *Error
	if errors.As(err, &access) {
		return err
	}
	return &StreamError{Err: err}
}

// ErrNotRunning means the conversation's Host is not running; a read never wakes it.
var ErrNotRunning = errors.New("the conversation's Host is not running")

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
func HostErrorCode(err *host.Error) (webapiproto.ErrorCode, int) {
	switch err.Kind {
	case host.Offline:
		return webapiproto.ErrorCodeDeviceOffline, 409
	case host.Unavailable:
		return webapiproto.ErrorCodeCloudUnavailable, 503
	case host.TooLarge:
		return webapiproto.ErrorCodeDirectoryTooLarge, 413
	case host.Failed:
		if err.Code != "" {
			switch err.Code {
			case "ENOENT":
				return webapiproto.ErrorCodeFsError, 404
			case "EACCES", "EPERM":
				return webapiproto.ErrorCodeFsError, 403
			}
		}
	case host.Protocol, host.Interrupted:
	}
	return webapiproto.ErrorCodeHostOperationFailed, 500
}
