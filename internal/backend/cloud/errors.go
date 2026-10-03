package cloud

import (
	"fmt"
	"log/slog"

	"github.com/wspl/demi/internal/webapi"
)

// ErrorKind identifies a Cloud admission or transition failure.
type ErrorKind uint8

const (
	// Closed means the Cloud is shutting down.
	Closed ErrorKind = iota
	// CrashLoop means its runtime was lost too often in a short time.
	CrashLoop
	// AtCapacity means every cross-user capacity permit is held.
	AtCapacity
	// Resetting means another reset holds the Cloud.
	Resetting
	// Failed means a boot, save, reset, storage operation or manager call failed.
	Failed
)

// Error is why the Cloud admits no operation or a transition failed. Use
// errors.As to inspect Kind and errors.Is/As through Unwrap for its cause.
type Error struct {
	// Kind identifies the failure category.
	Kind ErrorKind
	// Err is the underlying failure for Failed, including its user-facing text.
	Err error
}

// Error returns the Rust CloudError message for this failure.
func (e *Error) Error() string {
	switch e.Kind {
	case Closed:
		return "Cloud is shutting down"
	case CrashLoop:
		return "Cloud repeatedly failed; reset the environment to recover"
	case AtCapacity:
		return "Cloud capacity is currently full; retry later"
	case Resetting:
		return "Another reset is in progress"
	default:
		return e.Err.Error()
	}
}

// Unwrap returns the underlying failure.
func (e *Error) Unwrap() error { return e.Err }

// Code returns the web error code and HTTP status for a refused operation.
func (e *Error) Code() (webapi.ErrorCode, int) {
	switch e.Kind {
	case Closed:
		return webapi.ErrorCodeBackendClosing, 503
	case CrashLoop:
		return webapi.ErrorCodeCloudCrashLoop, 503
	case AtCapacity:
		return webapi.ErrorCodeCloudCapacity, 503
	case Resetting:
		return webapi.ErrorCodeCloudResetting, 409
	default:
		return webapi.ErrorCodeCloudUnavailable, 503
	}
}

// ManagerErrorKind identifies why a call to the manager has no result.
type ManagerErrorKind uint8

const (
	// ManagerUnavailable means the manager could not be reached or the socket
	// dropped before it answered; whether the operation ran is unknown.
	ManagerUnavailable ManagerErrorKind = iota
	// ManagerFailed means the manager ran the operation and it failed.
	ManagerFailed
	// ManagerResult means the result does not match the operation's contract.
	ManagerResult
)

// ManagerError reports a manager call's failure, retaining its cause for
// errors.Is and errors.As. Err preserves the peer or transport diagnostic.
type ManagerError struct {
	// Kind distinguishes transport, operation and result failures.
	Kind ManagerErrorKind
	// Operation is the wire operation name for unavailable or unexpected results.
	Operation string
	// Err is the transport or decoding cause, or an error containing the
	// manager's failure reply verbatim.
	Err error
}

// Error returns the Rust MachinesError message for this failure.
func (e *ManagerError) Error() string {
	switch e.Kind {
	case ManagerUnavailable:
		return fmt.Sprintf("Machine manager unavailable during %s: %v", e.Operation, e.Err)
	case ManagerResult:
		return fmt.Sprintf("the machine manager answered %s with an unexpected result: %v", e.Operation, e.Err)
	default:
		return e.Err.Error()
	}
}

// Unwrap returns the underlying transport or decoding failure.
func (e *ManagerError) Unwrap() error { return e.Err }

// RecoveryErrorKind identifies why startup could not recover its Clouds.
type RecoveryErrorKind uint8

const (
	// RecoveryMachines means a manager operation failed.
	RecoveryMachines RecoveryErrorKind = iota
	// RecoveryStorage means a control storage operation failed.
	RecoveryStorage
	// RecoveryMissingDevice means a reset names a device that no longer exists.
	RecoveryMissingDevice
)

// RecoveryError is why the backend could not recover its Clouds before serving.
type RecoveryError struct {
	// Kind identifies the failed recovery step.
	Kind RecoveryErrorKind
	// Device is the missing device for RecoveryMissingDevice.
	Device webapi.DeviceID
	// Err is the manager or storage cause for the other kinds.
	Err error
}

// Error returns the Rust RecoveryError message for this failure.
func (e *RecoveryError) Error() string {
	if e.Kind == RecoveryMissingDevice {
		return fmt.Sprintf("a reset names the device %s, which no longer exists", e.Device)
	}
	return e.Err.Error()
}

// Unwrap returns the underlying manager or storage failure.
func (e *RecoveryError) Unwrap() error { return e.Err }

// failed preserves the cause of a Cloud transition failure.
func failed(err error) error {
	if err == nil {
		return nil
	}
	return &Error{Kind: Failed, Err: err}
}

// storageFailed preserves the storage cause with the Cloud's user-facing diagnostic.
func storageFailed(err error) error {
	if err == nil {
		return nil
	}
	slog.Error("the Cloud's records failed", "error", err)
	//nolint:staticcheck // Preserve Rust user-facing text verbatim.
	return failed(fmt.Errorf("The Cloud's records could not be read or written: %w", err))
}
