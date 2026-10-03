//go:build unix

package backendtest

import (
	"errors"

	"golang.org/x/sys/unix"
)

// ProcessRunning reports whether the process pid still exists, as a scenario
// checks that a job's process ended with its job. A process another user owns
// exists too.
func ProcessRunning(pid int) (bool, error) {
	err := unix.Kill(pid, 0)
	switch {
	case err == nil, errors.Is(err, unix.EPERM):
		return true, nil
	case errors.Is(err, unix.ESRCH):
		return false, nil
	default:
		return false, err
	}
}
