//go:build linux

package sandbox

import (
	"errors"
	"fmt"
	"syscall"
)

var (
	// ErrAllowlist means a boot names a different configured backend.
	//nolint:staticcheck // User-visible text, kept byte for byte.
	ErrAllowlist = errors.New(
		"Cloud backend differs from configured allowlist",
	)
	// ErrNotGrowable means a recovered boot has no owned loop device numbers.
	//nolint:staticcheck // User-visible text, kept byte for byte.
	ErrNotGrowable = errors.New(
		"Cloud volume growth needs the running sandbox's loop devices",
	)
	// ErrWriters means the cgroup did not empty after its writers were killed.
	//nolint:staticcheck // User-visible text, kept byte for byte.
	ErrWriters = errors.New(
		"Cloud runtime writers did not terminate",
	)
)

// exitDescription preserves the recovery diagnostic's process status.
func exitDescription(status syscall.WaitStatus) string {
	if status.Exited() {
		return fmt.Sprintf("exit status: %d", status.ExitStatus())
	}
	return fmt.Sprintf("signal: %d (%s)", status.Signal(), status.Signal())
}
