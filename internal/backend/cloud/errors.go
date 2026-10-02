package cloud

import "github.com/wspl/demi/internal/webapi"

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
func (e *Error) Error() string { panic("not written: b-cloud") }

// Unwrap returns the underlying failure.
func (e *Error) Unwrap() error { panic("not written: b-cloud") }

// Code returns the web error code and HTTP status for a refused operation.
func (e *Error) Code() (webapi.ErrorCode, int) { panic("not written: b-cloud") }

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
func (e *ManagerError) Error() string { panic("not written: b-cloud") }

// Unwrap returns the underlying transport or decoding failure.
func (e *ManagerError) Unwrap() error { panic("not written: b-cloud") }

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
func (e *RecoveryError) Error() string { panic("not written: b-cloud") }

// Unwrap returns the underlying manager or storage failure.
func (e *RecoveryError) Unwrap() error { panic("not written: b-cloud") }
