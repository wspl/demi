//go:build windows

package commandservice

import (
	"errors"
	"syscall"
)

// The Windows errors of a pipe whose peer closed it, which package syscall
// does not all name.
const (
	errorNoData           = syscall.Errno(232) // ERROR_NO_DATA
	errorPipeNotConnected = syscall.Errno(233) // ERROR_PIPE_NOT_CONNECTED
)

// closedByPeer reports whether err is what the operating system says of a
// connection whose peer closed it: a write into a pipe that has no reader, or a
// pipe whose other end is gone.
func closedByPeer(err error) bool {
	return errors.Is(err, syscall.ERROR_BROKEN_PIPE) ||
		errors.Is(err, errorNoData) ||
		errors.Is(err, errorPipeNotConnected)
}
