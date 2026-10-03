//go:build linux

package sandbox

import (
	"errors"
	"fmt"
	"strings"
	"syscall"

	"github.com/wspl/demi/internal/machinewire"
)

var (
	// ErrAllowlist means a boot names a different configured backend.
	//nolint:staticcheck // Preserve the Rust product error verbatim.
	ErrAllowlist = errors.New(
		"Cloud backend differs from configured allowlist",
	)
	// ErrNotGrowable means a recovered boot has no owned loop device numbers.
	//nolint:staticcheck // Preserve the Rust product error verbatim.
	ErrNotGrowable = errors.New(
		"Cloud volume growth needs the running sandbox's loop devices",
	)
	// ErrWriters means the cgroup did not empty after its writers were killed.
	//nolint:staticcheck // Preserve the Rust product error verbatim.
	ErrWriters = errors.New(
		"Cloud runtime writers did not terminate",
	)
)

// NeedsRecoveryError identifies a filesystem e2fsck could not recover.
type NeedsRecoveryError struct {
	Volume  machinewire.Volume
	Message string
}

// Error describes the volume and filesystem-check output.
func (e *NeedsRecoveryError) Error() string {
	return fmt.Sprintf("Cloud %s filesystem needs recovery: %s", e.Volume, e.Message)
}

// SignalError reports a failed termination request for a still-live sandbox.
type SignalError struct{ Message string }

// Error describes the failed sandbox signal.
func (e *SignalError) Error() string {
	return "Cannot signal Cloud sandbox: " + e.Message
}

// ListError reports malformed runsc list output.
type ListError struct{ Source error }

// Error describes the runtime inspection failure.
func (e *ListError) Error() string {
	return "Cannot inspect Cloud runtimes: " + e.Source.Error()
}

// Unwrap preserves the decoding error.
func (e *ListError) Unwrap() error {
	return e.Source
}

// StartError contains the tail of the failed sandbox's runtime log.
type StartError struct{ Message string }

// Error describes the failed sandbox start.
func (e *StartError) Error() string {
	return "Cloud start failed: " + e.Message
}

// MissingControllersError names every required controller absent from cgroup v2.
type MissingControllersError struct{ Missing []string }

// Error names the missing controllers and the setting for running without limits.
func (e *MissingControllersError) Error() string {
	return "Cloud resource limits need the cgroup v2 cpu, memory and pids controllers " +
		"at /sys/fs/cgroup; missing: " + strings.Join(
		e.Missing,
		", ",
	) + ". DEMI_MANAGED_LIMITS=off runs Clouds without limits"
}

// OwnerError reports an invalid saved-namespace owner record.
type OwnerError struct {
	Path   string
	Source error
}

// Error describes the invalid record.
func (e *OwnerError) Error() string {
	return fmt.Sprintf("%s is not a valid namespace owner record: %v", e.Path, e.Source)
}

// Unwrap preserves the record decoding failure.
func (e *OwnerError) Unwrap() error {
	return e.Source
}

// OtherOwnerError identifies a saved namespace belonging to a different state directory.
type OtherOwnerError struct{ Data string }

// Error requests recovery using the original state directory.
func (e *OtherOwnerError) Error() string {
	return "Recover the previous Cloud manager with its original state directory: " + e.Data
}

// RecoveryError reports a recovery child that exited unsuccessfully.
type RecoveryError struct{ Status syscall.WaitStatus }

// Error describes the recovery child's exit status.
func (e *RecoveryError) Error() string {
	return fmt.Sprintf("Cloud namespace recovery failed: %s", exitDescription(e.Status))
}

// exitDescription preserves the recovery diagnostic's process status.
func exitDescription(status syscall.WaitStatus) string {
	if status.Exited() {
		return fmt.Sprintf("exit status: %d", status.ExitStatus())
	}
	return fmt.Sprintf("signal: %d (%s)", status.Signal(), status.Signal())
}
