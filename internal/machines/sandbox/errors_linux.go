//go:build linux

package sandbox

import (
	"errors"
	"syscall"

	"github.com/wspl/demi/internal/machinewire"
)

var (
	// ErrAllowlist means a boot names a different configured backend.
	ErrAllowlist = errors.New("Cloud backend differs from configured allowlist") //nolint:staticcheck // Preserve the Rust product error verbatim.
	// ErrNotGrowable means a recovered boot has no owned loop device numbers.
	ErrNotGrowable = errors.New("Cloud volume growth needs the running sandbox's loop devices") //nolint:staticcheck // Preserve the Rust product error verbatim.
	// ErrWriters means the cgroup did not empty after its writers were killed.
	ErrWriters = errors.New("Cloud runtime writers did not terminate") //nolint:staticcheck // Preserve the Rust product error verbatim.
)

// NeedsRecoveryError identifies a filesystem e2fsck could not recover.
type NeedsRecoveryError struct {
	Volume  machinewire.Volume
	Message string
}

// Error describes the volume and filesystem-check output.
func (e *NeedsRecoveryError) Error() string { panic("not written: m-sandbox") }

// SignalError reports a failed termination request for a still-live sandbox.
type SignalError struct{ Message string }

// Error describes the failed sandbox signal.
func (e *SignalError) Error() string { panic("not written: m-sandbox") }

// ListError reports malformed runsc list output.
type ListError struct{ Source error }

// Error describes the runtime inspection failure.
func (e *ListError) Error() string { panic("not written: m-sandbox") }

// Unwrap preserves the decoding error.
func (e *ListError) Unwrap() error { panic("not written: m-sandbox") }

// StartError contains the tail of the failed sandbox's runtime log.
type StartError struct{ Message string }

// Error describes the failed sandbox start.
func (e *StartError) Error() string { panic("not written: m-sandbox") }

// MissingControllersError names every required controller absent from cgroup v2.
type MissingControllersError struct{ Missing []string }

// Error names the missing controllers and the setting for running without limits.
func (e *MissingControllersError) Error() string { panic("not written: m-sandbox") }

// OwnerError reports an invalid saved-namespace owner record.
type OwnerError struct {
	Path   string
	Source error
}

// Error describes the invalid record.
func (e *OwnerError) Error() string { panic("not written: m-sandbox") }

// Unwrap preserves the record decoding failure.
func (e *OwnerError) Unwrap() error { panic("not written: m-sandbox") }

// OtherOwnerError identifies a saved namespace belonging to a different state directory.
type OtherOwnerError struct{ Data string }

// Error requests recovery using the original state directory.
func (e *OtherOwnerError) Error() string { panic("not written: m-sandbox") }

// RecoveryError reports a recovery child that exited unsuccessfully.
type RecoveryError struct{ Status syscall.WaitStatus }

// Error describes the recovery child's exit status.
func (e *RecoveryError) Error() string { panic("not written: m-sandbox") }
